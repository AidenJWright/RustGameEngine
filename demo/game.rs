//! Game entry point with launcher-first multiplayer flow.
//!
//! Run commands:
//!   `cargo run --bin game`                      (new UI launcher flow)
//!   `cargo run --bin game -- single`            (single player)
//!   `cargo run --bin game -- host Alice 127.0.0.1:7101` (legacy)
//!   `cargo run --bin game -- join 1234 Bob 127.0.0.1:7102` (legacy)

#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::too_many_lines)]

use std::collections::HashSet;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use forge_ecs::app::AppCore;
use forge_ecs::components::{Color, Shape, Tag, Transform, Velocity};
use forge_ecs::ecs::entity::Entity;
use forge_ecs::ecs::resource::{DeltaTime, ElapsedTime};
use forge_ecs::ecs::world::World;
use forge_ecs::math::Vec3;
use forge_ecs::messaging::{LoopPhase, MessageBus};
use forge_ecs::multiplayer;
use forge_ecs::multiplayer::matchmaking::{self, LobbyState, MatchEvent, MatchRequest};
use forge_ecs::multiplayer::{apply_snapshot, InputFrame, MatchSession, MatchState, NetworkEvent};
use forge_ecs::platform::{map_window_event, KeyCode, PlatformEvent};
use forge_ecs::renderer::draw::DrawCommand;
use forge_ecs::systems::{MovementSystem, SinusoidSystem};

#[derive(Debug, Parser)]
#[command(
    name = "game",
    about = "Game runtime with launcher-first multiplayer flow"
)]
struct Cli {
    /// Matchmaker server address used by launcher and host/join modes.
    #[arg(long, default_value = "127.0.0.1:7000")]
    matchmaker: String,

    #[command(subcommand)]
    mode: Option<Mode>,
}

#[derive(Debug, Subcommand, Clone)]
enum Mode {
    /// Run single-player locally.
    Single,
    /// Legacy mode: create a lobby and become host without launcher UI.
    Host {
        /// Display name in match events.
        player_name: String,
        /// Advertised gameplay endpoint for direct gameplay transport.
        game_addr: String,
    },
    /// Legacy mode: join a lobby and wait for start without launcher UI.
    Join {
        /// Lobby code.
        lobby_code: String,
        /// Display name in match events.
        player_name: String,
        /// Advertised gameplay endpoint for direct gameplay transport.
        game_addr: String,
    },
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct InputState {
    left: bool,
    right: bool,
    up: bool,
    down: bool,
}

impl InputState {
    fn movement_axis(&self) -> (f32, f32) {
        let x = (if self.left { -1.0_f32 } else { 0.0_f32 })
            + (if self.right { 1.0_f32 } else { 0.0_f32 });
        let y = (if self.up { -1.0_f32 } else { 0.0_f32 })
            + (if self.down { 1.0_f32 } else { 0.0_f32 });

        let len = (x * x + y * y).sqrt();
        if (len > 1.0_f32) && len > 0.0 {
            (x / len, y / len)
        } else {
            (x, y)
        }
    }

    fn apply(&mut self, event: PlatformEvent) {
        match event {
            PlatformEvent::KeyPressed(KeyCode::Left) => self.left = true,
            PlatformEvent::KeyReleased(KeyCode::Left) => self.left = false,
            PlatformEvent::KeyPressed(KeyCode::Right) => self.right = true,
            PlatformEvent::KeyReleased(KeyCode::Right) => self.right = false,
            PlatformEvent::KeyPressed(KeyCode::Up) => self.up = true,
            PlatformEvent::KeyReleased(KeyCode::Up) => self.up = false,
            PlatformEvent::KeyPressed(KeyCode::Down) => self.down = true,
            PlatformEvent::KeyReleased(KeyCode::Down) => self.down = false,
            _ => {}
        }
    }
}

#[derive(Debug)]
struct MultiplayerRuntime {
    session: MatchSession,
    tick_accumulator: f32,
}

#[derive(Debug, Clone, Copy)]
struct PlayerColor {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

const PLAYER_COLORS: [PlayerColor; 4] = [
    PlayerColor {
        r: 1.0,
        g: 0.2,
        b: 0.2,
        a: 1.0,
    },
    PlayerColor {
        r: 0.2,
        g: 0.6,
        b: 1.0,
        a: 1.0,
    },
    PlayerColor {
        r: 0.3,
        g: 0.9,
        b: 0.3,
        a: 1.0,
    },
    PlayerColor {
        r: 1.0,
        g: 1.0,
        b: 0.2,
        a: 1.0,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LauncherScreen {
    Connect,
    LobbyChoice,
    CreateLobbyConfig,
    JoinLobbyCode,
    WaitingRoom,
}

#[derive(Debug, Clone, Copy)]
enum PendingRequest {
    Ping { sent_at: Instant },
    CreateLobby { sent_at: Instant },
    JoinLobby { sent_at: Instant },
    StartMatch { sent_at: Instant },
}

impl PendingRequest {
    fn sent_at(self) -> Instant {
        match self {
            Self::Ping { sent_at }
            | Self::CreateLobby { sent_at }
            | Self::JoinLobby { sent_at }
            | Self::StartMatch { sent_at } => sent_at,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Ping { .. } => "connect",
            Self::CreateLobby { .. } => "create lobby",
            Self::JoinLobby { .. } => "join lobby",
            Self::StartMatch { .. } => "start match",
        }
    }
}

struct LauncherRuntime {
    screen: LauncherScreen,
    username_input: String,
    matchmaker_input: String,
    join_code_input: String,
    target_players: u8,
    status_message: String,
    error_message: String,
    connected_matchmaker: Option<SocketAddr>,
    control_socket: Option<UdpSocket>,
    prebound_game_socket: Option<UdpSocket>,
    local_game_addr: Option<SocketAddr>,
    local_player_id: Option<u64>,
    lobby_code: Option<String>,
    lobby_state: Option<LobbyState>,
    pending_request: Option<PendingRequest>,
    last_heartbeat: Instant,
}

impl LauncherRuntime {
    fn new(default_matchmaker: String) -> Self {
        Self {
            screen: LauncherScreen::Connect,
            username_input: "Player".to_string(),
            matchmaker_input: default_matchmaker,
            join_code_input: String::new(),
            target_players: 2,
            status_message: "Enter username and matchmaker address.".to_string(),
            error_message: String::new(),
            connected_matchmaker: None,
            control_socket: None,
            prebound_game_socket: None,
            local_game_addr: None,
            local_player_id: None,
            lobby_code: None,
            lobby_state: None,
            pending_request: None,
            last_heartbeat: Instant::now(),
        }
    }

    fn is_host(&self) -> bool {
        let Some(local_player_id) = self.local_player_id else {
            return false;
        };
        let Some(lobby) = self.lobby_state.as_ref() else {
            return false;
        };
        lobby.host_client_id == Some(local_player_id)
    }

    fn pending_label(&self) -> Option<&'static str> {
        self.pending_request.map(PendingRequest::label)
    }

    fn connect(&mut self) {
        self.error_message.clear();

        if self.username_input.trim().is_empty() {
            self.error_message = "Username cannot be empty.".to_string();
            return;
        }

        if self.pending_request.is_some() {
            return;
        }

        let server_addr = match self.matchmaker_input.parse::<SocketAddr>() {
            Ok(addr) => addr,
            Err(error) => {
                self.error_message = format!("Invalid matchmaker address: {error}");
                return;
            }
        };

        let socket = match bind_control_socket(server_addr) {
            Ok(socket) => socket,
            Err(error) => {
                self.error_message = format!("Could not bind control socket: {error}");
                return;
            }
        };

        if let Err(error) = socket.set_nonblocking(true) {
            self.error_message = format!("Could not set nonblocking mode: {error}");
            return;
        }

        self.control_socket = Some(socket);
        self.connected_matchmaker = Some(server_addr);

        if let Err(error) = self.send_request(MatchRequest::Ping) {
            self.error_message = format!("Connect request failed: {error}");
            self.pending_request = None;
            return;
        }

        self.pending_request = Some(PendingRequest::Ping {
            sent_at: Instant::now(),
        });
        self.status_message = format!("Connecting to {server_addr}...");
    }

    fn create_lobby(&mut self) {
        self.error_message.clear();

        if self.pending_request.is_some() {
            return;
        }

        let Some(server_addr) = self.connected_matchmaker else {
            self.error_message = "Not connected to a matchmaker.".to_string();
            self.screen = LauncherScreen::Connect;
            return;
        };

        if !(1..=4).contains(&self.target_players) {
            self.error_message = "Target players must be between 1 and 4.".to_string();
            return;
        }

        let (game_socket, local_addr) = match prebind_gameplay_socket(server_addr) {
            Ok(values) => values,
            Err(error) => {
                self.error_message = format!("Could not bind gameplay socket: {error}");
                return;
            }
        };

        let request = MatchRequest::CreateLobby {
            player_name: self.username_input.trim().to_string(),
            game_addr: local_addr.to_string(),
            target_players: self.target_players,
        };

        if let Err(error) = self.send_request(request) {
            self.error_message = format!("Create lobby request failed: {error}");
            return;
        }

        self.prebound_game_socket = Some(game_socket);
        self.local_game_addr = Some(local_addr);
        self.pending_request = Some(PendingRequest::CreateLobby {
            sent_at: Instant::now(),
        });
        self.status_message = "Creating lobby...".to_string();
    }

    fn join_lobby(&mut self) {
        self.error_message.clear();

        if self.pending_request.is_some() {
            return;
        }

        let Some(server_addr) = self.connected_matchmaker else {
            self.error_message = "Not connected to a matchmaker.".to_string();
            self.screen = LauncherScreen::Connect;
            return;
        };

        let sanitized_code = self
            .join_code_input
            .chars()
            .filter(|ch| ch.is_ascii_digit())
            .collect::<String>();
        if sanitized_code.len() != 4 {
            self.error_message = "Lobby code must be exactly 4 digits.".to_string();
            return;
        }

        let (game_socket, local_addr) = match prebind_gameplay_socket(server_addr) {
            Ok(values) => values,
            Err(error) => {
                self.error_message = format!("Could not bind gameplay socket: {error}");
                return;
            }
        };

        let request = MatchRequest::JoinLobby {
            lobby_code: sanitized_code.clone(),
            player_name: self.username_input.trim().to_string(),
            game_addr: local_addr.to_string(),
        };

        if let Err(error) = self.send_request(request) {
            self.error_message = format!("Join lobby request failed: {error}");
            return;
        }

        self.join_code_input = sanitized_code;
        self.prebound_game_socket = Some(game_socket);
        self.local_game_addr = Some(local_addr);
        self.pending_request = Some(PendingRequest::JoinLobby {
            sent_at: Instant::now(),
        });
        self.status_message = "Joining lobby...".to_string();
    }

    fn request_start_match(&mut self) {
        self.error_message.clear();

        if self.pending_request.is_some() {
            return;
        }

        if !self.is_host() {
            self.error_message = "Only the host can start the match.".to_string();
            return;
        }

        let Some(lobby_code) = self.lobby_code.clone() else {
            self.error_message = "No active lobby.".to_string();
            return;
        };
        let Some(client_id) = self.local_player_id else {
            self.error_message = "Missing local player id.".to_string();
            return;
        };

        if let Err(error) = self.send_request(MatchRequest::StartMatch {
            lobby_code,
            client_id,
        }) {
            self.error_message = format!("Start request failed: {error}");
            return;
        }

        self.pending_request = Some(PendingRequest::StartMatch {
            sent_at: Instant::now(),
        });
        self.status_message = "Start request sent.".to_string();
    }

    fn update(&mut self) -> Option<MatchSession> {
        self.check_pending_timeout();
        self.maybe_send_heartbeat();

        loop {
            let event = {
                let Some(socket) = self.control_socket.as_ref() else {
                    return None;
                };

                let mut buffer = [0_u8; 65_536];
                match socket.recv_from(&mut buffer) {
                    Ok((size, _from_addr)) => {
                        match matchmaking::deserialize_request::<MatchEvent>(&buffer[..size]) {
                            Ok(event) => Some(event),
                            Err(error) => {
                                self.error_message = format!("Invalid matchmaker packet: {error}");
                                None
                            }
                        }
                    }
                    Err(error)
                        if error.kind() == io::ErrorKind::WouldBlock
                            || error.kind() == io::ErrorKind::TimedOut =>
                    {
                        break;
                    }
                    Err(error) => {
                        self.error_message = format!("Matchmaker receive failed: {error}");
                        break;
                    }
                }
            };

            let Some(event) = event else {
                continue;
            };

            if let Some(session) = self.handle_event(event) {
                return Some(session);
            }
        }

        None
    }

    fn handle_event(&mut self, event: MatchEvent) -> Option<MatchSession> {
        match event {
            MatchEvent::Pong => {
                if matches!(self.pending_request, Some(PendingRequest::Ping { .. })) {
                    self.pending_request = None;
                    self.screen = LauncherScreen::LobbyChoice;
                    self.status_message =
                        "Connected. Choose Create Lobby or Join Lobby.".to_string();
                }
            }
            MatchEvent::LobbyCreated {
                lobby_code,
                player_id,
                lobby,
            } => {
                self.pending_request = None;
                self.local_player_id = Some(player_id);
                self.lobby_code = Some(lobby_code.clone());
                self.lobby_state = Some(lobby);
                self.screen = LauncherScreen::WaitingRoom;
                self.last_heartbeat = Instant::now();
                self.status_message = format!("Lobby {lobby_code} created. Waiting for players.");
            }
            MatchEvent::LobbyJoined {
                lobby_code,
                player_id,
                lobby,
            } => {
                self.pending_request = None;
                self.local_player_id = Some(player_id);
                self.lobby_code = Some(lobby_code.clone());
                self.lobby_state = Some(lobby);
                self.screen = LauncherScreen::WaitingRoom;
                self.last_heartbeat = Instant::now();
                self.status_message = format!("Joined lobby {lobby_code}. Waiting for start.");
            }
            MatchEvent::LobbyUpdated { lobby_code, lobby } => {
                if self.lobby_code.as_deref() == Some(lobby_code.as_str()) {
                    self.lobby_state = Some(lobby);
                }
            }
            MatchEvent::MatchStart {
                lobby_code,
                host_client_id,
                seed,
                player_endpoints,
            } => {
                let Some(local_player_id) = self.local_player_id else {
                    self.error_message =
                        "MatchStart received before local player id assignment.".to_string();
                    return None;
                };
                if self.lobby_code.as_deref() != Some(lobby_code.as_str()) {
                    return None;
                }

                let Some(game_socket) = self.prebound_game_socket.take() else {
                    self.error_message =
                        "MatchStart received but gameplay socket is unavailable.".to_string();
                    return None;
                };

                let state = MatchState {
                    lobby_code,
                    host_peer_id: host_client_id,
                    shared_seed: seed,
                    players: player_endpoints,
                    start_tick: 0,
                };

                match MatchSession::new_with_socket(
                    multiplayer::net_types::NetworkPolicy::default(),
                    state,
                    local_player_id,
                    game_socket,
                ) {
                    Ok(session) => return Some(session),
                    Err(error) => {
                        self.error_message = format!("Could not start gameplay session: {error}");
                    }
                }
            }
            MatchEvent::Error { message } => {
                self.pending_request = None;
                self.error_message = message;
            }
        }

        None
    }

    fn check_pending_timeout(&mut self) {
        const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

        let Some(pending) = self.pending_request else {
            return;
        };

        if pending.sent_at().elapsed() > REQUEST_TIMEOUT {
            self.pending_request = None;
            self.error_message = format!("{} timed out.", pending.label());
        }
    }

    fn maybe_send_heartbeat(&mut self) {
        if self.screen != LauncherScreen::WaitingRoom {
            return;
        }

        if self.last_heartbeat.elapsed() < Duration::from_secs(5) {
            return;
        }

        let Some(lobby_code) = self.lobby_code.clone() else {
            return;
        };
        let Some(client_id) = self.local_player_id else {
            return;
        };

        let request = MatchRequest::Heartbeat {
            lobby_code,
            client_id,
            game_addr: self.local_game_addr.map(|addr| addr.to_string()),
        };

        if let Err(error) = self.send_request(request) {
            self.error_message = format!("Heartbeat failed: {error}");
        }

        self.last_heartbeat = Instant::now();
    }

    fn send_request(&self, request: MatchRequest) -> io::Result<()> {
        let socket = self.control_socket.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "control socket not initialized",
            )
        })?;
        let server = self.connected_matchmaker.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "matchmaker address not configured",
            )
        })?;

        let payload = matchmaking::serialize_request(&request)?;
        socket.send_to(&payload, server)?;
        Ok(())
    }
}

struct DemoState {
    core: AppCore,
    bus: MessageBus,
    last_time: Instant,
    player_entities: Vec<(u64, Entity)>,
    player_slots: Vec<(u64, usize)>,
    local_player_id: u64,
    input_state: InputState,
    multiplayer: Option<MultiplayerRuntime>,
    launcher: Option<LauncherRuntime>,
    scene_initialized: bool,
}

enum StartupMode {
    Launcher { matchmaker_addr: String },
    Single,
    LegacySession(MatchSession),
}

struct GameApp {
    startup: Option<StartupMode>,
    state: Option<DemoState>,
}

impl ApplicationHandler for GameApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }

        let attrs = WindowAttributes::default()
            .with_title("Forge ECS -- Game")
            .with_inner_size(PhysicalSize::new(1280_u32, 720_u32))
            .with_resizable(true);

        let window: Window = event_loop
            .create_window(attrs)
            .expect("failed to create window");
        let core = AppCore::from_window(window).expect("AppCore creation failed");

        let mut bus = MessageBus::new();
        bus.register(LoopPhase::Update, 0, SinusoidSystem);
        bus.register(LoopPhase::Update, 10, MovementSystem);

        let startup = self
            .startup
            .take()
            .expect("startup mode should be set before resumed");

        let mut state = DemoState {
            core,
            bus,
            last_time: Instant::now(),
            player_entities: Vec::new(),
            player_slots: Vec::new(),
            local_player_id: 0,
            input_state: InputState::default(),
            multiplayer: None,
            launcher: None,
            scene_initialized: false,
        };

        match startup {
            StartupMode::Launcher { matchmaker_addr } => {
                let _ = state
                    .core
                    .platform
                    .window
                    .set_title("Forge ECS -- Multiplayer Launcher");
                state.launcher = Some(LauncherRuntime::new(matchmaker_addr));
            }
            StartupMode::Single => {
                initialize_single_player_scene(&mut state);
            }
            StartupMode::LegacySession(session) => {
                initialize_multiplayer_scene(&mut state, session);
            }
        }

        self.state = Some(state);
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        let Some(s) = &mut self.state else {
            return;
        };
        let full = winit::event::Event::<()>::NewEvents(cause);
        s.core.imgui.handle_event(s.core.platform.window(), &full);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(s) = &mut self.state else {
            return;
        };
        if window_id != s.core.platform.window.id() {
            return;
        }

        s.core
            .imgui
            .handle_window_event(s.core.platform.window(), window_id, &event);

        match &event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                s.core.render_ctx.resize(size.width, size.height);
                if s.scene_initialized {
                    reposition_players(
                        &mut s.core.world,
                        &s.player_entities,
                        &s.player_slots,
                        size.width as f32,
                        size.height as f32,
                    );
                }
            }
            WindowEvent::RedrawRequested => render(s),
            _ => {}
        }

        if s.launcher.is_none() {
            if let Some(platform_event) = map_window_event(&event) {
                s.input_state.apply(platform_event);
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        let Some(s) = &mut self.state else {
            return;
        };

        s.core.imgui.handle_about_to_wait(s.core.platform.window());

        let now = Instant::now();
        let dt = now.duration_since(s.last_time).as_secs_f32();
        s.last_time = now;

        if let Some(resource) = s.core.world.resource_mut::<DeltaTime>() {
            resource.0 = dt;
        }
        if let Some(resource) = s.core.world.resource_mut::<ElapsedTime>() {
            resource.0 += dt;
        }

        if let Some(launcher) = s.launcher.as_mut() {
            if let Some(session) = launcher.update() {
                initialize_multiplayer_scene(s, session);
                s.launcher = None;
            }
        }

        if s.scene_initialized {
            if let Some(multiplayer) = s.multiplayer.as_mut() {
                apply_multiplayer_tick(
                    multiplayer,
                    dt,
                    &mut s.core.world,
                    s.local_player_id,
                    &s.player_entities,
                    &s.input_state,
                );
            } else if let Some(entity) = s.player_entities.iter().find_map(|(id, entity)| {
                if *id == s.local_player_id {
                    Some(*entity)
                } else {
                    None
                }
            }) {
                apply_local_velocity(&mut s.core.world, entity, &s.input_state);
            }

            s.bus.run_frame(&mut s.core.world);
        }

        s.core.platform.window.request_redraw();
    }
}

fn bind_control_socket(server_addr: SocketAddr) -> io::Result<UdpSocket> {
    if server_addr.is_ipv4() {
        UdpSocket::bind("0.0.0.0:0")
    } else {
        UdpSocket::bind("[::]:0")
    }
}

fn prebind_gameplay_socket(matchmaker_addr: SocketAddr) -> io::Result<(UdpSocket, SocketAddr)> {
    let probe = if matchmaker_addr.is_ipv4() {
        UdpSocket::bind("0.0.0.0:0")?
    } else {
        UdpSocket::bind("[::]:0")?
    };

    let _ = probe.connect(matchmaker_addr);
    let mut local_ip = probe.local_addr()?.ip();
    if local_ip.is_unspecified() {
        local_ip = if matchmaker_addr.is_ipv4() {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else {
            IpAddr::V6(Ipv6Addr::LOCALHOST)
        };
    }

    let gameplay_socket = UdpSocket::bind(SocketAddr::new(local_ip, 0))?;
    let local_addr = gameplay_socket.local_addr()?;
    Ok((gameplay_socket, local_addr))
}

fn initialize_single_player_scene(state: &mut DemoState) {
    let (window_width, window_height) = current_viewport(&state.core);
    let (player_entities, local_player_id) =
        setup_single_player_scene(&mut state.core.world, window_width, window_height);

    state.player_slots = player_entities
        .iter()
        .enumerate()
        .map(|(slot, (player_id, _))| (*player_id, slot))
        .collect();
    state.player_entities = player_entities;
    state.local_player_id = local_player_id;
    state.multiplayer = None;
    state.scene_initialized = true;

    let _ = state
        .core
        .platform
        .window
        .set_title("Forge ECS -- Game [singleplayer]");
}

fn initialize_multiplayer_scene(state: &mut DemoState, session: MatchSession) {
    let (window_width, window_height) = current_viewport(&state.core);
    let player_entities =
        setup_multiplayer_scene(&mut state.core.world, &session, window_width, window_height);
    let local_player_id = session.local_peer_id();

    state.player_slots = player_entities
        .iter()
        .enumerate()
        .map(|(slot, (player_id, _))| (*player_id, slot))
        .collect();
    state.player_entities = player_entities;
    state.local_player_id = local_player_id;
    state.multiplayer = Some(MultiplayerRuntime {
        session,
        tick_accumulator: 0.0,
    });
    state.scene_initialized = true;

    let is_host = state
        .multiplayer
        .as_ref()
        .map(|runtime| runtime.session.is_host())
        .unwrap_or(false);
    let _ = state.core.platform.window.set_title(&format!(
        "Forge ECS -- Game [peer {} {}]",
        local_player_id,
        if is_host { "host" } else { "client" }
    ));
}

fn current_viewport(core: &AppCore) -> (f32, f32) {
    let width = if core.render_ctx.surface_config.width == 0 {
        1280.0
    } else {
        core.render_ctx.surface_config.width as f32
    };
    let height = if core.render_ctx.surface_config.height == 0 {
        720.0
    } else {
        core.render_ctx.surface_config.height as f32
    };
    (width, height)
}

fn draw_launcher_ui(ui: &imgui::Ui, launcher: &mut LauncherRuntime) {
    ui.window("Multiplayer Launcher")
        .size([520.0, 430.0], imgui::Condition::Always)
        .position([24.0, 24.0], imgui::Condition::Always)
        .build(|| {
            ui.text("Launch Flow");
            ui.separator();

            match launcher.screen {
                LauncherScreen::Connect => {
                    ui.text("Connect to a running matchmaker server.");
                    ui.input_text("Username", &mut launcher.username_input)
                        .build();
                    ui.input_text("Matchmaker", &mut launcher.matchmaker_input)
                        .build();

                    if ui.button("Connect") {
                        launcher.connect();
                    }
                }
                LauncherScreen::LobbyChoice => {
                    ui.text("Connected. Choose a lobby action.");
                    if let Some(addr) = launcher.connected_matchmaker {
                        ui.text(format!("Matchmaker: {addr}"));
                    }

                    if ui.button("Create Lobby") {
                        launcher.screen = LauncherScreen::CreateLobbyConfig;
                        launcher.error_message.clear();
                    }
                    if ui.button("Join Lobby") {
                        launcher.screen = LauncherScreen::JoinLobbyCode;
                        launcher.error_message.clear();
                    }
                }
                LauncherScreen::CreateLobbyConfig => {
                    ui.text("Select lobby size.");

                    let mut target = i32::from(launcher.target_players);
                    ui.slider("Players", 1_i32, 4_i32, &mut target);
                    launcher.target_players = target as u8;

                    if ui.button("Create") {
                        launcher.create_lobby();
                    }
                    if ui.button("Back") {
                        launcher.screen = LauncherScreen::LobbyChoice;
                        launcher.error_message.clear();
                    }
                }
                LauncherScreen::JoinLobbyCode => {
                    ui.text("Enter a 4-digit lobby code.");
                    ui.input_text("Lobby Code", &mut launcher.join_code_input)
                        .build();

                    if ui.button("Join") {
                        launcher.join_lobby();
                    }
                    if ui.button("Back") {
                        launcher.screen = LauncherScreen::LobbyChoice;
                        launcher.error_message.clear();
                    }
                }
                LauncherScreen::WaitingRoom => {
                    let lobby = launcher.lobby_state.as_ref();
                    let lobby_code = launcher.lobby_code.as_deref().unwrap_or("----");
                    ui.text(format!("Lobby Code: {lobby_code}"));

                    if let Some(lobby) = lobby {
                        ui.text(format!(
                            "Players: {}/{}",
                            lobby.players.len(),
                            lobby.target_players
                        ));
                        if let Some(countdown) = lobby.countdown_seconds {
                            ui.text(format!("Auto-start in: {countdown}s"));
                        } else {
                            ui.text("Auto-start waiting for required players...");
                        }

                        ui.separator();
                        ui.text("Joined Players");

                        let mut players = lobby.players.clone();
                        players.sort_by_key(|player| player.client_id);
                        for player in players {
                            let host_tag = if Some(player.client_id) == lobby.host_client_id {
                                " (Host)"
                            } else {
                                ""
                            };
                            ui.bullet_text(format!("{}{}", player.name, host_tag));
                        }
                    }

                    ui.separator();
                    if launcher.is_host() {
                        if ui.button("Start Game") {
                            launcher.request_start_match();
                        }
                    } else {
                        ui.text_disabled("Start Game (Host Only)");
                    }
                }
            }

            if let Some(label) = launcher.pending_label() {
                ui.separator();
                ui.text(format!("Pending: {label}"));
            }

            if !launcher.status_message.is_empty() {
                ui.separator();
                ui.text(format!("Status: {}", launcher.status_message));
            }

            if !launcher.error_message.is_empty() {
                ui.text_colored(
                    [1.0, 0.35, 0.35, 1.0],
                    format!("Error: {}", launcher.error_message),
                );
            }
        });
}

fn reposition_players(
    world: &mut World,
    player_entities: &[(u64, Entity)],
    player_slots: &[(u64, usize)],
    window_width: f32,
    window_height: f32,
) {
    let player_count = player_entities.len().min(4);
    player_slots.iter().for_each(|(peer_id, slot)| {
        let entity =
            player_entities
                .iter()
                .find_map(|(id, entity)| if id == peer_id { Some(*entity) } else { None });
        if let Some(entity) = entity {
            if let Some(transform) = world.get_mut::<Transform>(entity) {
                transform.position =
                    spawn_position(*slot, player_count, window_width, window_height);
            }
        }
    });
}

fn setup_single_player_scene(
    world: &mut World,
    window_width: f32,
    window_height: f32,
) -> (Vec<(u64, Entity)>, u64) {
    let scene_root = world.spawn();
    world.insert(scene_root, Tag::new("scene_root"));

    let color = PLAYER_COLORS[0];
    let position = spawn_position(0, 1, window_width, window_height);
    let circle_entity = world.spawn_child(scene_root);
    world.insert(
        circle_entity,
        Transform {
            position,
            ..Transform::identity()
        },
    );
    world.insert(circle_entity, Shape::Circle { radius: 50.0 });
    world.insert(
        circle_entity,
        Color {
            r: color.r,
            g: color.g,
            b: color.b,
            a: color.a,
        },
    );
    world.insert(circle_entity, Velocity { dx: 0.0, dy: 0.0 });

    (vec![(1, circle_entity)], 1)
}

fn setup_multiplayer_scene(
    world: &mut World,
    session: &MatchSession,
    window_width: f32,
    window_height: f32,
) -> Vec<(u64, Entity)> {
    let scene_root = world.spawn();
    world.insert(scene_root, Tag::new("scene_root"));

    let mut players = session.players().to_vec();
    players.sort_by_key(|player| player.client_id);
    let player_count = players.len().min(4);

    let player_entities: Vec<(u64, Entity)> = players
        .iter()
        .take(player_count)
        .enumerate()
        .map(|(index, player)| {
            let position = spawn_position(index, player_count, window_width, window_height);
            let color = PLAYER_COLORS[index % PLAYER_COLORS.len()];
            let circle_entity = world.spawn_child(scene_root);
            world.insert(
                circle_entity,
                Transform {
                    position,
                    ..Transform::identity()
                },
            );
            world.insert(circle_entity, Shape::Circle { radius: 50.0 });
            world.insert(
                circle_entity,
                Color {
                    r: color.r,
                    g: color.g,
                    b: color.b,
                    a: color.a,
                },
            );
            world.insert(circle_entity, Velocity { dx: 0.0, dy: 0.0 });
            (player.client_id, circle_entity)
        })
        .collect();

    let unique_entity_count = player_entities
        .iter()
        .map(|(_, entity)| *entity)
        .collect::<HashSet<_>>()
        .len();
    if unique_entity_count != player_entities.len() {
        println!(
            "warning: duplicated entity IDs in multiplayer spawn ({:?})",
            player_entities
                .iter()
                .map(|(_, entity)| format!("{entity:?}"))
                .collect::<Vec<_>>()
        );
    }

    player_entities
}

fn spawn_position(
    player_index: usize,
    total_players: usize,
    window_width: f32,
    window_height: f32,
) -> Vec3 {
    match total_players {
        1 => Vec3::new(window_width * 0.5, window_height * 0.5, 0.0),
        2 => {
            let x = if player_index == 0 {
                window_width * 0.25
            } else {
                window_width * 0.75
            };
            Vec3::new(x, window_height * 0.5, 0.0)
        }
        _ => {
            let is_left = player_index % 2 == 0;
            let is_top = player_index < 2;
            let x = if is_left {
                window_width * 0.25
            } else {
                window_width * 0.75
            };
            let y = if is_top {
                window_height * 0.25
            } else {
                window_height * 0.75
            };
            Vec3::new(x, y, 0.0)
        }
    }
}

fn apply_local_velocity(world: &mut World, entity: Entity, input_state: &InputState) {
    let move_speed = 220.0;
    let (move_x, move_y) = input_state.movement_axis();
    if let Some(velocity) = world.get_mut::<Velocity>(entity) {
        velocity.dx = move_x * move_speed;
        velocity.dy = move_y * move_speed;
    }
}

fn apply_player_velocity(
    world: &mut World,
    player_entities: &[(u64, Entity)],
    player_id: u64,
    move_x: f32,
    move_y: f32,
) -> bool {
    let move_speed = 220.0;
    let maybe_entity = player_entities
        .iter()
        .find(|(id, _)| *id == player_id)
        .map(|(_, entity)| *entity);

    if let Some(entity) = maybe_entity {
        if let Some(velocity) = world.get_mut::<Velocity>(entity) {
            velocity.dx = move_x * move_speed;
            velocity.dy = move_y * move_speed;
            return true;
        }
    }
    false
}

fn apply_multiplayer_tick(
    runtime: &mut MultiplayerRuntime,
    delta: f32,
    world: &mut World,
    local_player_id: u64,
    player_entities: &[(u64, Entity)],
    input_state: &InputState,
) {
    let tick_rate = runtime.session.tick_rate().max(1);
    let tick_dt = 1.0_f32 / tick_rate as f32;
    runtime.tick_accumulator += delta;

    while runtime.tick_accumulator >= tick_dt {
        runtime.tick_accumulator -= tick_dt;

        let (move_x, move_y) = input_state.movement_axis();
        let input = InputFrame {
            tick: runtime.session.current_tick(),
            player_id: runtime.session.local_peer_id(),
            move_x,
            move_y,
            action_bits: 0,
        };

        runtime.session.enqueue_local_input(input);
        let _ = apply_player_velocity(world, player_entities, local_player_id, move_x, move_y);

        runtime.session.tick();
        let events = runtime.session.drain_network_events();
        for event in events {
            match event {
                NetworkEvent::CorrectionReceived { snapshot, .. } => {
                    apply_snapshot(world, &snapshot);
                }
                NetworkEvent::InputReceived(input) => {
                    let InputFrame {
                        player_id,
                        move_x,
                        move_y,
                        ..
                    } = input;
                    let _ =
                        apply_player_velocity(world, player_entities, player_id, move_x, move_y);
                }
                NetworkEvent::HashMismatch { .. } => {}
            }
        }
    }
}

fn render(s: &mut DemoState) {
    let Some((surface_texture, view)) = s.core.render_ctx.begin_frame() else {
        return;
    };

    let mut encoder =
        s.core
            .render_ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("game frame"),
            });

    if s.scene_initialized {
        s.core.world.query3::<Transform, Shape, Color>().for_each(
            |(_, transform, shape, color)| {
                let cmd = make_draw_cmd(&transform, shape, color);
                s.core.draw_queue.push(cmd);
            },
        );
    }

    s.core.draw_queue.flush(
        &s.core.render_ctx,
        &view,
        &mut encoder,
        &s.core.circle_pipeline,
        &s.core.rect_pipeline,
        [0.1, 0.1, 0.1, 1.0],
    );

    if let Some(launcher) = s.launcher.as_mut() {
        let ui = s.core.imgui.begin_frame(s.core.platform.window());
        draw_launcher_ui(ui, launcher);
        s.core.imgui.end_frame(
            s.core.platform.window(),
            &s.core.render_ctx.device,
            &s.core.render_ctx.queue,
            &mut encoder,
            &view,
        );
    }

    s.core
        .render_ctx
        .queue
        .submit(std::iter::once(encoder.finish()));
    s.core.render_ctx.end_frame(surface_texture);
}

fn make_draw_cmd(transform: &Transform, shape: &Shape, color: &Color) -> DrawCommand {
    match shape {
        Shape::Circle { radius } => DrawCommand::Circle {
            x: transform.position.x,
            y: transform.position.y,
            radius: *radius,
            color: [color.r, color.g, color.b, color.a],
        },
        Shape::Rect { width, height } => DrawCommand::Rect {
            x: transform.position.x,
            y: transform.position.y,
            width: *width,
            height: *height,
            color: [color.r, color.g, color.b, color.a],
        },
    }
}

fn bootstrap_session(mode: &Mode, matchmaker_addr: &str) -> io::Result<Option<MatchSession>> {
    let addr = matchmaker_addr.parse::<SocketAddr>().map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid matchmaker address: {error}"),
        )
    })?;

    match mode {
        Mode::Single => Ok(None),
        Mode::Host {
            player_name,
            game_addr,
        } => {
            let socket = UdpSocket::bind("0.0.0.0:0").expect("could not bind UDP socket");
            socket
                .set_read_timeout(Some(Duration::from_millis(350)))
                .expect("set read timeout failed");

            let create = send_matchmaker_request(
                &socket,
                &addr,
                MatchRequest::CreateLobby {
                    player_name: player_name.clone(),
                    game_addr: game_addr.clone(),
                    target_players: 4,
                },
            )?;

            let (lobby_code, local_player_id) = match create {
                MatchEvent::LobbyCreated {
                    lobby_code,
                    player_id,
                    ..
                } => (lobby_code, player_id),
                other => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unexpected response while creating lobby: {other:?}"),
                    ));
                }
            };

            println!(
                "host lobby created: code={lobby_code}, player={local_player_id}, game_addr={game_addr}"
            );
            let state = await_match_start(
                &socket,
                &addr,
                &lobby_code,
                local_player_id,
                true,
                Some(game_addr.clone()),
            )?;
            Ok(Some(MatchSession::new(
                multiplayer::net_types::NetworkPolicy::default(),
                state,
                local_player_id,
            )?))
        }
        Mode::Join {
            lobby_code,
            player_name,
            game_addr,
        } => {
            let socket = UdpSocket::bind("0.0.0.0:0").expect("could not bind UDP socket");
            socket
                .set_read_timeout(Some(Duration::from_millis(350)))
                .expect("set read timeout failed");

            let join = send_matchmaker_request(
                &socket,
                &addr,
                MatchRequest::JoinLobby {
                    lobby_code: lobby_code.clone(),
                    player_name: player_name.clone(),
                    game_addr: game_addr.clone(),
                },
            )?;

            let local_player_id = match join {
                MatchEvent::LobbyJoined {
                    player_id,
                    lobby_code: _,
                    ..
                } => player_id,
                other => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unexpected response while joining lobby: {other:?}"),
                    ));
                }
            };

            println!(
                "joined lobby: code={lobby_code}, player={local_player_id}, game_addr={game_addr}"
            );
            let state = await_match_start(
                &socket,
                &addr,
                lobby_code,
                local_player_id,
                false,
                Some(game_addr.clone()),
            )?;
            Ok(Some(MatchSession::new(
                multiplayer::net_types::NetworkPolicy::default(),
                state,
                local_player_id,
            )?))
        }
    }
}

fn await_match_start(
    socket: &UdpSocket,
    matchmaker_addr: &SocketAddr,
    lobby_code: &str,
    local_player_id: u64,
    is_host: bool,
    game_addr: Option<String>,
) -> io::Result<MatchState> {
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut last_start_send = Instant::now();
    let mut last_heartbeat = Instant::now();

    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "match start wait timed out",
            ));
        }

        if is_host && last_start_send.elapsed() > Duration::from_secs(1) {
            let _ = send_no_reply(
                socket,
                matchmaker_addr,
                MatchRequest::StartMatch {
                    lobby_code: lobby_code.to_string(),
                    client_id: local_player_id,
                },
            );
            last_start_send = Instant::now();
        }

        if last_heartbeat.elapsed() > Duration::from_secs(5) {
            let request = MatchRequest::Heartbeat {
                lobby_code: lobby_code.to_string(),
                client_id: local_player_id,
                game_addr: game_addr.clone(),
            };
            let _ = send_no_reply(socket, matchmaker_addr, request);
            last_heartbeat = Instant::now();
        }

        match recv_match_event(socket) {
            Ok(MatchEvent::MatchStart {
                lobby_code: started_code,
                host_client_id,
                seed,
                player_endpoints,
            }) => {
                if started_code == lobby_code {
                    return Ok(MatchState {
                        lobby_code: started_code,
                        host_peer_id: host_client_id,
                        shared_seed: seed,
                        players: player_endpoints,
                        start_tick: 0,
                    });
                }
            }
            Ok(MatchEvent::LobbyUpdated {
                lobby_code: updated_code,
                lobby,
            }) => {
                if updated_code == lobby_code {
                    println!(
                        "lobby {updated_code} players={}/{} countdown={:?}",
                        lobby.players.len(),
                        lobby.target_players,
                        lobby.countdown_seconds
                    );
                }
            }
            Ok(MatchEvent::Error { message }) => {
                println!("matchmaker error: {message}");
            }
            Ok(MatchEvent::Pong) => {}
            Ok(other) => {
                println!("ignored matchmaker event: {other:?}");
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) => {}
            Err(error) => return Err(error),
        }
    }
}

fn send_matchmaker_request(
    socket: &UdpSocket,
    server: &SocketAddr,
    request: MatchRequest,
) -> io::Result<MatchEvent> {
    send_no_reply(socket, server, request.clone())?;
    recv_match_event(socket)
}

fn send_no_reply(socket: &UdpSocket, server: &SocketAddr, request: MatchRequest) -> io::Result<()> {
    let payload = matchmaking::serialize_request(&request)?;
    let _ = socket.send_to(&payload, server)?;
    Ok(())
}

fn recv_match_event(socket: &UdpSocket) -> io::Result<MatchEvent> {
    let mut buffer = [0_u8; 65_536];
    let (size, _) = socket.recv_from(&mut buffer)?;
    matchmaking::deserialize_request::<MatchEvent>(&buffer[..size])
}

fn main() {
    let cli = Cli::parse();

    let startup = match cli.mode.clone() {
        None => StartupMode::Launcher {
            matchmaker_addr: cli.matchmaker.clone(),
        },
        Some(Mode::Single) => StartupMode::Single,
        Some(mode) => match bootstrap_session(&mode, &cli.matchmaker) {
            Ok(Some(session)) => StartupMode::LegacySession(session),
            Ok(None) => StartupMode::Single,
            Err(error) => {
                eprintln!("could not initialize multiplayer mode: {error}");
                std::process::exit(1);
            }
        },
    };

    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut app = GameApp {
        startup: Some(startup),
        state: None,
    };
    event_loop.run_app(&mut app).expect("event loop error");
}
