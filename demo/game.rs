//! Game entry point with launcher-first multiplayer flow.
//!
//! Run commands:
//!   `cargo run --bin game`                      (new UI launcher flow)
//!   `cargo run --bin game -- single`            (single player)
//!   `cargo run --bin game -- host Alice`        (legacy, auto game addr)
//!   `cargo run --bin game -- join 1234 Bob`     (legacy, auto game addr)
//!   `cargo run --bin game -- --game-port 7010`  (override gameplay UDP port)

#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::too_many_lines)]

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{MouseScrollDelta, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use forge_ecs::app::AppCore;
use forge_ecs::components::{Camera, Color, PlayerInput, Shape, SpawnPoints, Tag, Transform, Velocity};
use forge_ecs::scene::reload_scene;
use forge_ecs::ecs::entity::Entity;
use forge_ecs::ecs::resource::{DeltaTime, ElapsedTime, KeysPressed};
use forge_ecs::ecs::world::World;
use forge_ecs::game::ctf::resources::{
    ControlState, CtfInputState, CtfPointerState, CtfSyncState,
};
use forge_ecs::game::ctf::setup::{ctf_camera_entity, setup_ctf_scene_entities};
use forge_ecs::game::ctf::systems::hud::draw_hud as draw_ctf_hud;
use forge_ecs::game::ctf::systems::{
    AutoMoveSystem, CtfInputSystem, FlagCarrySystem, FlagMotionSystem, FlagPickupSystem,
    StopOnWinSystem, WallCollisionSystem, WinConditionSystem,
};
use forge_ecs::game::ctf::{
    ACTION_RESTART, ACTION_SWITCH, ACTION_TAG, RESTART_KEY, SWITCH_KEY,
};
use forge_ecs::math::Vec3;
use forge_ecs::messaging::{LoopPhase, MessageBus};
use forge_ecs::multiplayer;
use forge_ecs::multiplayer::matchmaking::{
    self, CtfSlot, CtfSlotAssignment, GameMode, LobbyState, MatchEvent, MatchRequest,
};
use forge_ecs::multiplayer::{
    apply_snapshot, capture_snapshot, state_hash, CtfPointerInput, InputFrame, MatchSession,
    MatchState, NetworkEvent, NetworkResource, DEFAULT_SNAPSHOT_STRIDE,
};
use forge_ecs::platform::{
    map_window_event, KeyCode, MouseButton as EngineMouseButton, PlatformEvent,
};
use forge_ecs::renderer::draw::DrawCommand;
use forge_ecs::systems::{MovementSystem, PlayerInputSystem, SinusoidSystem};

const DEFAULT_GAMEPLAY_PORT: u16 = 7001;

#[derive(Debug, Parser)]
#[command(
    name = "game",
    about = "Game runtime with launcher-first multiplayer flow"
)]
struct Cli {
    /// Matchmaker server address used by launcher and host/join modes.
    ///
    /// Override with the `MATCHMAKER_ADDR` environment variable to target a
    /// remote matchmaker without passing a CLI flag every run.
    #[arg(long, default_value = "127.0.0.1:7000", env = "MATCHMAKER_ADDR")]
    matchmaker: String,
    /// Local UDP gameplay port advertised to peers (launcher + legacy auto-addr).
    #[arg(long, default_value_t = DEFAULT_GAMEPLAY_PORT)]
    game_port: u16,

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
        /// Optional advertised gameplay endpoint, e.g. 192.168.1.10:7001.
        ///
        /// If omitted, this is auto-derived from `--game-port`.
        game_addr: Option<String>,
    },
    /// Legacy mode: join a lobby and wait for start without launcher UI.
    Join {
        /// Lobby code.
        lobby_code: String,
        /// Display name in match events.
        player_name: String,
        /// Optional advertised gameplay endpoint, e.g. 192.168.1.11:7001.
        ///
        /// If omitted, this is auto-derived from `--game-port`.
        game_addr: Option<String>,
    },
}

/// Map engine `KeyCode` to the discriminant used by `PlayerInput` and `KeysPressed`.
///
/// Discriminants must match `forge_ecs::systems::player_input::config_key_discriminant`.
fn key_discriminant(code: KeyCode) -> Option<u32> {
    match code {
        KeyCode::Left => Some(0),
        KeyCode::Right => Some(1),
        KeyCode::Up => Some(2),
        KeyCode::Down => Some(3),
        KeyCode::W => Some(4),
        KeyCode::A => Some(5),
        KeyCode::S => Some(6),
        KeyCode::D => Some(7),
        KeyCode::Space => Some(8),
        KeyCode::Return => Some(9),
        KeyCode::LeftShift => Some(10),
        KeyCode::R => Some(11),
        _ => None,
    }
}

/// Compute normalised (move_x, move_y) from a player entity's `PlayerInput`
/// bindings and the current `KeysPressed` resource.
fn compute_movement(keys: &KeysPressed, pi: &PlayerInput) -> (f32, f32) {
    use forge_ecs::systems::player_input::config_key_discriminant;
    let neg_h = keys.is_held(config_key_discriminant(pi.horizontal.negative));
    let pos_h = keys.is_held(config_key_discriminant(pi.horizontal.positive));
    let neg_v = keys.is_held(config_key_discriminant(pi.vertical.negative));
    let pos_v = keys.is_held(config_key_discriminant(pi.vertical.positive));

    let raw_x = (if pos_h { 1.0_f32 } else { 0.0 }) - (if neg_h { 1.0_f32 } else { 0.0 });
    let raw_y = (if pos_v { 1.0_f32 } else { 0.0 }) - (if neg_v { 1.0_f32 } else { 0.0 });

    let len = (raw_x * raw_x + raw_y * raw_y).sqrt();
    if len > 1.0 {
        (raw_x / len, raw_y / len)
    } else {
        (raw_x, raw_y)
    }
}

/// Compute normalised (move_x, move_y) using default arrow-key bindings.
/// Used when the player entity has no `PlayerInput` component.
fn compute_movement_default(keys: &KeysPressed) -> (f32, f32) {
    let left = keys.is_held(0);
    let right = keys.is_held(1);
    let up = keys.is_held(2);
    let down = keys.is_held(3);

    let raw_x = (if right { 1.0_f32 } else { 0.0 }) - (if left { 1.0_f32 } else { 0.0 });
    let raw_y = (if down { 1.0_f32 } else { 0.0 }) - (if up { 1.0_f32 } else { 0.0 });

    let len = (raw_x * raw_x + raw_y * raw_y).sqrt();
    if len > 1.0 {
        (raw_x / len, raw_y / len)
    } else {
        (raw_x, raw_y)
    }
}

fn compute_ctf_input(keys: &KeysPressed) -> (f32, f32, u8) {
    let left = keys.is_held(0) || keys.is_held(5);
    let right = keys.is_held(1) || keys.is_held(7);
    let up = keys.is_held(2) || keys.is_held(4);
    let down = keys.is_held(3) || keys.is_held(6);

    let raw_x = (if right { 1.0_f32 } else { 0.0 }) - (if left { 1.0_f32 } else { 0.0 });
    let raw_y = (if down { 1.0_f32 } else { 0.0 }) - (if up { 1.0_f32 } else { 0.0 });
    let len = (raw_x * raw_x + raw_y * raw_y).sqrt();
    let (move_x, move_y) = if len > 1.0 {
        (raw_x / len, raw_y / len)
    } else {
        (raw_x, raw_y)
    };

    let mut action_bits = 0_u8;
    if keys.is_held(8) || keys.is_held(9) {
        action_bits |= ACTION_TAG;
    }
    if keys.is_held(SWITCH_KEY) {
        action_bits |= ACTION_SWITCH;
    }
    if keys.is_held(RESTART_KEY) {
        action_bits |= ACTION_RESTART;
    }

    (move_x, move_y, action_bits)
}

fn take_ctf_pointer_input(world: &mut World) -> Option<CtfPointerInput> {
    let pointer = world.resource_mut::<CtfPointerState>()?;
    Some(CtfPointerInput {
        aim_world: pointer.cursor_world,
        click_world: pointer.pending_left_click_world.take(),
    })
}

#[derive(Debug)]
struct MultiplayerRuntime {
    session: MatchSession,
    tick_accumulator: f32,
    /// Counts ticks since the last hash broadcast (host only).
    hash_check_counter: u32,
}

/// How many ticks between host hash broadcasts for desync detection.
const HASH_CHECK_INTERVAL: u32 = 30;

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
    SinglePlayerReady,
}

#[derive(Debug, Clone, Copy)]
enum PendingRequest {
    Ping { sent_at: Instant },
    CreateLobby { sent_at: Instant },
    JoinLobby { sent_at: Instant },
    StartMatch { sent_at: Instant },
    UpdateCtfAssignments { sent_at: Instant },
}

impl PendingRequest {
    fn sent_at(self) -> Instant {
        match self {
            Self::Ping { sent_at }
            | Self::CreateLobby { sent_at }
            | Self::JoinLobby { sent_at }
            | Self::StartMatch { sent_at }
            | Self::UpdateCtfAssignments { sent_at } => sent_at,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Ping { .. } => "connect",
            Self::CreateLobby { .. } => "create lobby",
            Self::JoinLobby { .. } => "join lobby",
            Self::StartMatch { .. } => "start match",
            Self::UpdateCtfAssignments { .. } => "update CTF assignments",
        }
    }
}

struct LauncherRuntime {
    screen: LauncherScreen,
    username_input: String,
    matchmaker_input: String,
    join_code_input: String,
    target_players: u8,
    game_mode: GameMode,
    status_message: String,
    error_message: String,
    connected_matchmaker: Option<SocketAddr>,
    control_socket: Option<UdpSocket>,
    prebound_game_socket: Option<UdpSocket>,
    local_game_addr: Option<SocketAddr>,
    gameplay_port: u16,
    local_player_id: Option<u64>,
    lobby_code: Option<String>,
    lobby_state: Option<LobbyState>,
    pending_request: Option<PendingRequest>,
    last_heartbeat: Instant,
    /// Set when the user has requested single-player mode; checked in `about_to_wait`.
    single_player_ready: bool,
    /// Cached connection details for the reconnect button.
    last_lobby_code: Option<String>,
}

impl LauncherRuntime {
    fn new(default_matchmaker: String, gameplay_port: u16) -> Self {
        Self {
            screen: LauncherScreen::Connect,
            username_input: "Player".to_string(),
            matchmaker_input: default_matchmaker,
            join_code_input: String::new(),
            target_players: 2,
            game_mode: GameMode::DefaultScene,
            status_message: "Enter username and matchmaker address.".to_string(),
            error_message: String::new(),
            connected_matchmaker: None,
            control_socket: None,
            prebound_game_socket: None,
            local_game_addr: None,
            gameplay_port,
            local_player_id: None,
            lobby_code: None,
            lobby_state: None,
            pending_request: None,
            last_heartbeat: Instant::now(),
            single_player_ready: false,
            last_lobby_code: None,
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

        let min_players = if self.game_mode == GameMode::CaptureTheFlag { 2 } else { 1 };
        if !(min_players..=4).contains(&self.target_players) {
            self.error_message =
                format!("Target players must be between {min_players} and 4.");
            return;
        }

        let (game_socket, local_addr) =
            match prebind_gameplay_socket(server_addr, self.gameplay_port) {
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
            game_mode: self.game_mode,
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

        let (game_socket, local_addr) =
            match prebind_gameplay_socket(server_addr, self.gameplay_port) {
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

    fn update_ctf_assignments(&mut self, assignments: Vec<CtfSlotAssignment>) {
        self.error_message.clear();

        if self.pending_request.is_some() {
            return;
        }

        if !self.is_host() {
            self.error_message = "Only the host can update CTF slots.".to_string();
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

        if let Err(error) = self.send_request(MatchRequest::UpdateCtfAssignments {
            lobby_code,
            client_id,
            assignments,
        }) {
            self.error_message = format!("CTF assignment update failed: {error}");
            return;
        }

        self.pending_request = Some(PendingRequest::UpdateCtfAssignments {
            sent_at: Instant::now(),
        });
        self.status_message = "CTF assignments updated.".to_string();
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
                    if matches!(
                        self.pending_request,
                        Some(PendingRequest::UpdateCtfAssignments { .. })
                    ) {
                        self.pending_request = None;
                    }
                    self.lobby_state = Some(lobby);
                }
            }
            MatchEvent::MatchStart {
                lobby_code,
                host_client_id,
                seed,
                player_endpoints,
                game_mode,
                ctf_assignments,
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
                    game_mode,
                    ctf_assignments,
                };

                // Cache the lobby code so the reconnect button can use it.
                self.last_lobby_code = Some(state.lobby_code.clone());

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
        const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

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
    multiplayer: Option<MultiplayerRuntime>,
    launcher: Option<LauncherRuntime>,
    scene_initialized: bool,
    game_mode: GameMode,
    /// Camera entity that tracks the local player's position.
    camera_entity: Option<Entity>,
    /// Manual camera offset from the followed player, controlled by middle-drag.
    camera_pan: Vec3,
    camera_middle_dragging: bool,
    camera_last_cursor: Option<(f64, f64)>,
    cursor_screen: Option<(f64, f64)>,
    /// When true the camera follows the locally-controlled entity each frame.
    /// Toggle with Left Control. Default on.
    follow_controlled_entity: bool,
}

enum StartupMode {
    Launcher {
        matchmaker_addr: String,
        gameplay_port: u16,
    },
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
            multiplayer: None,
            launcher: None,
            scene_initialized: false,
            game_mode: GameMode::DefaultScene,
            camera_entity: None,
            camera_pan: Vec3::ZERO,
            camera_middle_dragging: false,
            camera_last_cursor: None,
            cursor_screen: None,
            follow_controlled_entity: true,
        };

        match startup {
            StartupMode::Launcher {
                matchmaker_addr,
                gameplay_port,
            } => {
                let _ = state
                    .core
                    .platform
                    .window
                    .set_title("Forge ECS -- Multiplayer Launcher");
                state.launcher = Some(LauncherRuntime::new(matchmaker_addr, gameplay_port));
            }
            StartupMode::Single => {
                initialize_single_player_scene(&mut state);
            }
            StartupMode::LegacySession(session) => {
                if session.game_mode() == GameMode::CaptureTheFlag {
                    initialize_multiplayer_ctf_scene(&mut state, session);
                } else {
                    initialize_multiplayer_scene(&mut state, session);
                }
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
                // Do NOT reposition entities — the camera tracks the player,
                // so the viewport adjusts automatically on resize.
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let size = s.core.platform.window.inner_size();
                s.core.render_ctx.resize(size.width, size.height);
            }
            WindowEvent::RedrawRequested => render(s),
            _ => {}
        }

        // Update runtime input resources from raw keyboard/mouse events.
        // Keyboard drives both PlayerInputSystem and the multiplayer InputFrame.
        if s.launcher.is_none() {
            if let WindowEvent::MouseWheel { delta, .. } = &event {
                let wheel = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y,
                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 120.0,
                };
                if wheel != 0.0 {
                    if let Some(cam_entity) = s.camera_entity {
                        if let Some(camera) = s.core.world.get_mut::<Camera>(cam_entity) {
                            camera.zoom = (camera.zoom * (1.0 + wheel * 0.1)).clamp(0.05, 20.0);
                        }
                    }
                }
            }

            if let Some(platform_event) = map_window_event(&event) {
                match platform_event {
                    PlatformEvent::KeyPressed(code) => {
                        if code == KeyCode::ControlLeft {
                            s.follow_controlled_entity = !s.follow_controlled_entity;
                        }
                        if let Some(disc) = key_discriminant(code) {
                            if let Some(keys) = s.core.world.resource_mut::<KeysPressed>() {
                                keys.press(disc);
                            }
                        }
                    }
                    PlatformEvent::KeyReleased(code) => {
                        if let Some(disc) = key_discriminant(code) {
                            if let Some(keys) = s.core.world.resource_mut::<KeysPressed>() {
                                keys.release(disc);
                            }
                        }
                    }
                    PlatformEvent::MouseButton {
                        button: EngineMouseButton::Left,
                        pressed: true,
                    } => {
                        queue_ctf_click(s);
                    }
                    PlatformEvent::MouseButton {
                        button: EngineMouseButton::Middle,
                        pressed,
                    } => {
                        s.camera_middle_dragging = pressed;
                        s.camera_last_cursor = None;
                    }
                    PlatformEvent::MouseMoved { x, y } => {
                        s.cursor_screen = Some((x, y));
                        if s.camera_middle_dragging {
                            if let Some((last_x, last_y)) = s.camera_last_cursor {
                                let zoom = s
                                    .camera_entity
                                    .and_then(|entity| s.core.world.get::<Camera>(entity))
                                    .map_or(1.0, |camera| camera.zoom)
                                    .max(0.05);
                                let dx = (x - last_x) as f32 / zoom;
                                let dy = (y - last_y) as f32 / zoom;
                                if s.follow_controlled_entity {
                                    s.camera_pan.x -= dx;
                                    s.camera_pan.y -= dy;
                                } else if let Some(cam_entity) = s.camera_entity {
                                    if let Some(cam_tf) =
                                        s.core.world.get_mut::<Transform>(cam_entity)
                                    {
                                        cam_tf.position.x -= dx;
                                        cam_tf.position.y -= dy;
                                    }
                                }
                            }
                            s.camera_last_cursor = Some((x, y));
                        }
                        update_ctf_cursor_world(s, x, y);
                    }
                    _ => {}
                }
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
            if launcher.single_player_ready {
                s.launcher = None;
                initialize_single_player_scene(s);
            } else if let Some(session) = launcher.update() {
                if session.game_mode() == GameMode::CaptureTheFlag {
                    initialize_multiplayer_ctf_scene(s, session);
                } else {
                    initialize_multiplayer_scene(s, session);
                }
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
                    s.game_mode,
                );
            }
            // Single-player movement is handled by PlayerInputSystem via bus.

            s.bus.run_frame(&mut s.core.world);
            send_ctf_dirty_snapshot(s);

            // Move camera to follow the local player each frame.
            if s.follow_controlled_entity {
                if let Some(cam_entity) = s.camera_entity {
                    let player_pos = local_follow_position(s);
                    if let Some(pos) = player_pos {
                        if let Some(cam_tf) = s.core.world.get_mut::<Transform>(cam_entity) {
                            cam_tf.position = pos + s.camera_pan;
                        }
                    }
                }
            }
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

fn resolve_local_interface_ip(matchmaker_addr: SocketAddr) -> io::Result<IpAddr> {
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

    Ok(local_ip)
}

fn prebind_gameplay_socket(
    matchmaker_addr: SocketAddr,
    gameplay_port: u16,
) -> io::Result<(UdpSocket, SocketAddr)> {
    let local_ip = resolve_local_interface_ip(matchmaker_addr)?;
    // Use port 0 to let the OS assign an available port, preventing collisions
    // when multiple clients run on the same machine.  If an explicit port was
    // provided (non-default) we still honour it.
    let bind_port = if gameplay_port == DEFAULT_GAMEPLAY_PORT {
        0
    } else {
        gameplay_port
    };
    let gameplay_socket = UdpSocket::bind(SocketAddr::new(local_ip, bind_port))?;
    let local_addr = gameplay_socket.local_addr()?;
    Ok((gameplay_socket, local_addr))
}

fn local_follow_position(state: &DemoState) -> Option<Vec3> {
    if state.game_mode == GameMode::CaptureTheFlag {
        let selected_slot = state
            .core
            .world
            .resource::<ControlState>()?
            .controls
            .iter()
            .find(|control| control.client_id == state.local_player_id)?
            .selected_slot;
        let refs = state.core.world.resource::<forge_ecs::game::ctf::resources::EntityRefs>()?;
        return state
            .core
            .world
            .get::<Transform>(refs.player(selected_slot))
            .map(|tf| tf.position);
    }

    state
        .player_entities
        .iter()
        .find(|(id, _)| *id == state.local_player_id)
        .and_then(|(_, entity)| state.core.world.get::<Transform>(*entity).cloned())
        .map(|tf| tf.position)
}

fn send_ctf_dirty_snapshot(state: &mut DemoState) {
    if state.game_mode != GameMode::CaptureTheFlag {
        return;
    }
    let Some(multiplayer) = state.multiplayer.as_mut() else {
        return;
    };
    if !multiplayer.session.is_host() {
        return;
    }
    let dirty = state
        .core
        .world
        .resource::<CtfSyncState>()
        .is_some_and(|sync| sync.dirty);
    if !dirty {
        return;
    }

    let tick = multiplayer.session.current_tick();
    let snapshot = capture_snapshot(&state.core.world, tick);
    multiplayer.session.send_correction(tick, snapshot);
    state
        .core
        .world
        .insert_resource(CtfSyncState { dirty: false });
}

fn update_ctf_cursor_world(state: &mut DemoState, x: f64, y: f64) {
    if state.game_mode != GameMode::CaptureTheFlag || !state.scene_initialized {
        return;
    }
    let world_pos = screen_to_ctf_world(state, x, y);
    if let Some(pointer) = state.core.world.resource_mut::<CtfPointerState>() {
        pointer.cursor_world = Some(world_pos);
    }
}

fn queue_ctf_click(state: &mut DemoState) {
    if state.game_mode != GameMode::CaptureTheFlag || !state.scene_initialized {
        return;
    }
    let Some((x, y)) = state.cursor_screen else {
        return;
    };
    let world_pos = screen_to_ctf_world(state, x, y);
    if let Some(pointer) = state.core.world.resource_mut::<CtfPointerState>() {
        pointer.cursor_world = Some(world_pos);
        pointer.pending_left_click_world = Some(world_pos);
    }
}

fn screen_to_ctf_world(state: &DemoState, x: f64, y: f64) -> (f32, f32) {
    let screen_x = x as f32;
    let screen_y = y as f32;
    let width = state.core.render_ctx.surface_config.width as f32;
    let height = state.core.render_ctx.surface_config.height as f32;

    let Some(camera_entity) = state.camera_entity else {
        return (screen_x, screen_y);
    };
    let Some(camera_tf) = state.core.world.get::<Transform>(camera_entity) else {
        return (screen_x, screen_y);
    };
    let zoom = state
        .core
        .world
        .get::<Camera>(camera_entity)
        .map_or(1.0, |camera| camera.zoom)
        .max(0.05);

    (
        (screen_x - width * 0.5) / zoom + camera_tf.position.x,
        (screen_y - height * 0.5) / zoom + camera_tf.position.y,
    )
}

fn resolve_or_default_game_addr(
    provided_game_addr: Option<String>,
    matchmaker_addr: SocketAddr,
    gameplay_port: u16,
) -> io::Result<String> {
    if let Some(addr) = provided_game_addr {
        return Ok(addr);
    }

    let local_ip = resolve_local_interface_ip(matchmaker_addr)?;
    Ok(SocketAddr::new(local_ip, gameplay_port).to_string())
}

fn initialize_single_player_scene(state: &mut DemoState) {
    // Load scene.json; fall back to a generated default if missing.
    if let Err(e) = reload_scene(&mut state.core.world, "scene.json") {
        println!("scene.json not found or invalid ({e}), spawning default player");
        setup_single_player_scene(&mut state.core.world);
    }

    // Insert KeysPressed resource so PlayerInputSystem can function.
    state.core.world.insert_resource(KeysPressed::default());
    state
        .bus
        .register(LoopPhase::Update, PlayerInputSystem::PRIORITY, PlayerInputSystem);

    // Find the player entity by Tag "player"; fall back to any entity with Velocity.
    let player_entity = state
        .core
        .world
        .query::<Tag>()
        .find(|(_, tag)| tag.as_str() == "player")
        .map(|(e, _)| e)
        .or_else(|| state.core.world.query::<Velocity>().next().map(|(e, _)| e));

    let local_player_id = 1_u64;
    let player_entities: Vec<(u64, Entity)> = player_entity
        .map(|e| vec![(local_player_id, e)])
        .unwrap_or_default();

    state.player_slots = player_entities
        .iter()
        .enumerate()
        .map(|(slot, (player_id, _))| (*player_id, slot))
        .collect();
    state.player_entities = player_entities;
    state.local_player_id = local_player_id;
    state.multiplayer = None;
    state.scene_initialized = true;
    state.game_mode = GameMode::DefaultScene;
    state.camera_pan = Vec3::ZERO;
    state.camera_middle_dragging = false;
    state.camera_last_cursor = None;
    state.cursor_screen = None;
    state.follow_controlled_entity = true;

    // Find or create camera entity to follow the local player.
    state.camera_entity = find_or_create_camera(&mut state.core.world);

    let _ = state
        .core
        .platform
        .window
        .set_title("Forge ECS -- Game [singleplayer]");
}

fn initialize_multiplayer_scene(state: &mut DemoState, session: MatchSession) {
    // Load the shared editor scene first.
    if let Err(e) = reload_scene(&mut state.core.world, "scene.json") {
        println!("scene.json not found ({e}), using generated layout");
    }

    let local_player_id = session.local_peer_id();
    let mut players = session.players().to_vec();
    players.sort_by_key(|p| p.client_id);
    let scene_player_inputs = state.core.world.query::<PlayerInput>().count();

    // Find SpawnPoints entity in the loaded scene.
    let spawn_positions: Vec<Vec3> = state
        .core
        .world
        .query::<SpawnPoints>()
        .next()
        .map(|(_, sp)| {
            sp.positions
                .iter()
                .map(|p| Vec3::new(p[0], p[1], p[2]))
                .collect()
        })
        .unwrap_or_default();
    println!(
        "multiplayer scene init: local_peer={} host_peer={} players={} scene_player_inputs={} spawn_points={}",
        local_player_id,
        session.host_peer_id(),
        players.len(),
        scene_player_inputs,
        spawn_positions.len()
    );

    // Check capacity — warn if scene doesn't have enough spawn points.
    if !spawn_positions.is_empty() && players.len() > spawn_positions.len() {
        println!(
            "warning: {} players but only {} spawn points — extra players will share last slot",
            players.len(),
            spawn_positions.len()
        );
    }

    // Insert KeysPressed so the multiplayer tick can capture local input.
    state.core.world.insert_resource(KeysPressed::default());

    // Spawn a player entity for each peer at the corresponding spawn position.
    let player_entities: Vec<(u64, Entity)> = players
        .iter()
        .enumerate()
        .map(|(i, player)| {
            let position = if !spawn_positions.is_empty() {
                spawn_positions[i.min(spawn_positions.len() - 1)]
            } else {
                // Fallback: evenly distributed horizontal layout at world origin.
                let offset = (i as f32 - (players.len() as f32 - 1.0) * 0.5) * 200.0;
                Vec3::new(offset, 0.0, 0.0)
            };
            let color = PLAYER_COLORS[i % PLAYER_COLORS.len()];
            let entity = state.core.world.spawn();
            state.core.world.insert(entity, Transform { position, ..Transform::identity() });
            state.core.world.insert(entity, Shape::Circle { radius: 50.0 });
            state.core.world.insert(
                entity,
                Color { r: color.r, g: color.g, b: color.b, a: color.a },
            );
            state.core.world.insert(entity, Velocity { dx: 0.0, dy: 0.0 });
            state.core.world.insert(entity, Tag::new(&player.name));
            if player.client_id == local_player_id {
                state.core.world.insert(entity, PlayerInput::default());
            }
            println!(
                "multiplayer player entity: slot={i} player_id={} entity={} local={} position=({:.1}, {:.1}, {:.1})",
                player.client_id,
                entity,
                player.client_id == local_player_id,
                position.x,
                position.y,
                position.z
            );
            (player.client_id, entity)
        })
        .collect();

    state.player_slots = player_entities
        .iter()
        .enumerate()
        .map(|(slot, (player_id, _))| (*player_id, slot))
        .collect();
    state.player_entities = player_entities;
    state.local_player_id = local_player_id;

    // Register NetworkResource so game systems can read network events.
    let is_host = session.is_host();
    state.core.world.insert_resource(NetworkResource {
        pending_events: Vec::new(),
        local_peer_id: local_player_id,
        is_host,
        outbound_input: None,
    });

    state.multiplayer = Some(MultiplayerRuntime {
        session,
        tick_accumulator: 0.0,
        hash_check_counter: 0,
    });
    state.scene_initialized = true;
    state.game_mode = GameMode::DefaultScene;
    state.camera_pan = Vec3::ZERO;
    state.camera_middle_dragging = false;
    state.camera_last_cursor = None;
    state.cursor_screen = None;
    state.follow_controlled_entity = true;

    // Find or create camera entity to follow the local player.
    state.camera_entity = find_or_create_camera(&mut state.core.world);

    let _ = state.core.platform.window.set_title(&format!(
        "Forge ECS -- Game [peer {} {}]",
        local_player_id,
        if is_host { "host" } else { "client" }
    ));
}

fn initialize_multiplayer_ctf_scene(state: &mut DemoState, session: MatchSession) {
    reload_scene(&mut state.core.world, "assets/ctf_scene.json")
        .unwrap_or_else(|error| panic!("failed to load CTF scene: {error}"));

    let local_player_id = session.local_peer_id();
    let assignments = session.ctf_assignments().to_vec();
    let refs = setup_ctf_scene_entities(&mut state.core.world, &assignments);

    state.core.world.insert_resource(KeysPressed::default());
    state.bus.register(LoopPhase::Update, -20, CtfInputSystem);
    state.bus.register(LoopPhase::Update, -15, AutoMoveSystem);
    state.bus.register(LoopPhase::Update, -9, StopOnWinSystem);
    state.bus.register(LoopPhase::Update, 12, WallCollisionSystem);
    state.bus.register(LoopPhase::Update, 14, FlagMotionSystem);
    state.bus.register(LoopPhase::Update, 15, FlagPickupSystem);
    state.bus.register(LoopPhase::Update, 20, FlagCarrySystem);
    state.bus.register(LoopPhase::Update, 25, WinConditionSystem);

    let player_entities: Vec<(u64, Entity)> = assignments
        .iter()
        .map(|assignment| {
            (
                assignment.client_id,
                refs.player(assignment.primary_slot),
            )
        })
        .collect();

    state.player_slots = player_entities
        .iter()
        .enumerate()
        .map(|(slot, (player_id, _))| (*player_id, slot))
        .collect();
    state.player_entities = player_entities;
    state.local_player_id = local_player_id;

    let is_host = session.is_host();
    state.core.world.insert_resource(NetworkResource {
        pending_events: Vec::new(),
        local_peer_id: local_player_id,
        is_host,
        outbound_input: None,
    });

    state.multiplayer = Some(MultiplayerRuntime {
        session,
        tick_accumulator: 0.0,
        hash_check_counter: 0,
    });
    state.scene_initialized = true;
    state.game_mode = GameMode::CaptureTheFlag;
    state.camera_pan = Vec3::ZERO;
    state.camera_middle_dragging = false;
    state.camera_last_cursor = None;
    state.cursor_screen = None;
    state.follow_controlled_entity = true;
    state.camera_entity = Some(ctf_camera_entity(&state.core.world));

    let _ = state.core.platform.window.set_title(&format!(
        "Forge ECS -- Capture the Flag [peer {} {}]",
        local_player_id,
        if is_host { "host" } else { "client" }
    ));
}

/// Find an existing Camera entity in the world, or spawn a new one.
fn find_or_create_camera(world: &mut World) -> Option<Entity> {
    // Prefer an existing Camera entity placed in the scene by the editor.
    if let Some((entity, _)) = world.query::<Camera>().next() {
        return Some(entity);
    }
    // No camera in scene — create one so the game can render.
    let cam_entity = world.spawn();
    world.insert(cam_entity, Transform::identity());
    world.insert(cam_entity, Camera::new());
    Some(cam_entity)
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
                    ui.same_line();
                    if ui.button("Single Player") {
                        launcher.screen = LauncherScreen::SinglePlayerReady;
                    }

                    // Show Reconnect only if a previous session exists.
                    if let Some(last_code) = launcher.last_lobby_code.clone() {
                        ui.separator();
                        ui.text(format!("Previous lobby: {last_code}"));
                        if ui.button("Reconnect") {
                            launcher.join_code_input = last_code;
                            launcher.connect(); // Connect to matchmaker first.
                            launcher.screen = LauncherScreen::JoinLobbyCode;
                        }
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
                    ui.text("Select game mode and lobby size.");

                    let mut mode_index = if launcher.game_mode == GameMode::CaptureTheFlag {
                        1
                    } else {
                        0
                    };
                    let mode_labels = ["Default Scene", "Capture The Flag"];
                    if ui.combo_simple_string("Mode", &mut mode_index, &mode_labels) {
                        launcher.game_mode = if mode_index == 1 {
                            GameMode::CaptureTheFlag
                        } else {
                            GameMode::DefaultScene
                        };
                        if launcher.game_mode == GameMode::CaptureTheFlag {
                            launcher.target_players = launcher.target_players.clamp(2, 4);
                        }
                    }

                    let mut target = i32::from(launcher.target_players);
                    let min_players = if launcher.game_mode == GameMode::CaptureTheFlag {
                        2_i32
                    } else {
                        1_i32
                    };
                    ui.slider("Players", min_players, 4_i32, &mut target);
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
                LauncherScreen::SinglePlayerReady => {
                    ui.text("Single Player");
                    ui.text("Loads scene.json and starts the game locally.");
                    if ui.button("Play") {
                        launcher.single_player_ready = true;
                    }
                    if ui.button("Back") {
                        launcher.screen = LauncherScreen::Connect;
                    }
                }
                LauncherScreen::WaitingRoom => {
                    let lobby = launcher.lobby_state.clone();
                    let lobby_code = launcher.lobby_code.as_deref().unwrap_or("----");
                    ui.text(format!("Lobby Code: {lobby_code}"));

                    if let Some(lobby) = lobby {
                        ui.text(format!("Mode: {:?}", lobby.game_mode));
                        ui.text(format!(
                            "Players: {}/{}",
                            lobby.players.len(),
                            lobby.target_players
                        ));
                        if lobby.game_mode == GameMode::CaptureTheFlag {
                            ui.text("CTF starts manually after host slot assignment.");
                        } else if let Some(countdown) = lobby.countdown_seconds {
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

                        if lobby.game_mode == GameMode::CaptureTheFlag {
                            ui.separator();
                            draw_ctf_assignment_ui(ui, launcher, &lobby);
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

fn draw_ctf_assignment_ui(ui: &imgui::Ui, launcher: &mut LauncherRuntime, lobby: &LobbyState) {
    ui.text("CTF Slots");
    let mut players = lobby.players.clone();
    players.sort_by_key(|player| player.client_id);
    let mut assignments = normalized_ctf_assignments(lobby);
    let slot_labels = CtfSlot::ALL.map(CtfSlot::label);
    let is_host = launcher.is_host();
    let mut changed = false;

    for player in players {
        let assignment_index = assignments
            .iter()
            .position(|assignment| assignment.client_id == player.client_id)
            .expect("normalized assignment exists");
        let old_slot = assignments[assignment_index].primary_slot;
        let mut slot_index = old_slot.index();
        if is_host {
            let label = format!("{}##slot_{}", player.name, player.client_id);
            if ui.combo_simple_string(&label, &mut slot_index, &slot_labels) {
                let new_slot = CtfSlot::ALL[slot_index];
                if new_slot != old_slot {
                    if let Some(other) = assignments
                        .iter_mut()
                        .find(|assignment| assignment.primary_slot == new_slot)
                    {
                        other.primary_slot = old_slot;
                    }
                    assignments[assignment_index].primary_slot = new_slot;
                    changed = true;
                }
            }
        } else {
            ui.bullet_text(format!("{}: {}", player.name, old_slot.label()));
        }
    }

    if changed {
        launcher.update_ctf_assignments(assignments);
    }
}

fn normalized_ctf_assignments(lobby: &LobbyState) -> Vec<CtfSlotAssignment> {
    let mut assignments = lobby.ctf_assignments.clone();
    assignments.retain(|assignment| {
        lobby
            .players
            .iter()
            .any(|player| player.client_id == assignment.client_id)
    });

    for player in &lobby.players {
        if assignments
            .iter()
            .any(|assignment| assignment.client_id == player.client_id)
        {
            continue;
        }
        let slot = CtfSlot::ALL
            .iter()
            .copied()
            .find(|slot| {
                !assignments
                    .iter()
                    .any(|assignment| assignment.primary_slot == *slot)
            })
            .unwrap_or(CtfSlot::Red1);
        assignments.push(CtfSlotAssignment {
            client_id: player.client_id,
            primary_slot: slot,
        });
    }

    assignments.sort_by_key(|assignment| assignment.client_id);
    assignments
}


/// Spawn a minimal fallback player entity when `scene.json` is missing.
fn setup_single_player_scene(world: &mut World) {
    let scene_root = world.spawn();
    world.insert(scene_root, Tag::new("scene_root"));

    let color = PLAYER_COLORS[0];
    let circle_entity = world.spawn_child(scene_root);
    world.insert(circle_entity, Transform::identity());
    world.insert(circle_entity, Tag::new("player"));
    world.insert(circle_entity, Shape::Circle { radius: 50.0 });
    world.insert(
        circle_entity,
        Color { r: color.r, g: color.g, b: color.b, a: color.a },
    );
    world.insert(circle_entity, Velocity { dx: 0.0, dy: 0.0 });
    world.insert(circle_entity, PlayerInput::default());
}


fn apply_player_velocity(
    world: &mut World,
    player_entities: &[(u64, Entity)],
    player_id: u64,
    move_x: f32,
    move_y: f32,
) {
    let move_speed = 220.0;
    let maybe_entity = player_entities
        .iter()
        .find(|(id, _)| *id == player_id)
        .map(|(_, entity)| *entity);

    if let Some(entity) = maybe_entity {
        if let Some(velocity) = world.get_mut::<Velocity>(entity) {
            velocity.dx = move_x * move_speed;
            velocity.dy = move_y * move_speed;
        }
    }
}

fn apply_multiplayer_tick(
    runtime: &mut MultiplayerRuntime,
    delta: f32,
    world: &mut World,
    local_player_id: u64,
    player_entities: &[(u64, Entity)],
    game_mode: GameMode,
) {
    let tick_rate = runtime.session.tick_rate().max(1);
    let tick_dt = 1.0_f32 / tick_rate as f32;
    runtime.tick_accumulator += delta;

    while runtime.tick_accumulator >= tick_dt {
        runtime.tick_accumulator -= tick_dt;

        // Compute movement/action from local bindings + held keys.
        let (move_x, move_y, action_bits, ctf_pointer) = if game_mode == GameMode::CaptureTheFlag {
            let keys = world.resource::<KeysPressed>().cloned().unwrap_or_default();
            let (move_x, move_y, action_bits) = compute_ctf_input(&keys);
            (move_x, move_y, action_bits, take_ctf_pointer_input(world))
        } else {
            let keys = world.resource::<KeysPressed>().cloned().unwrap_or_default();
            let local_entity = player_entities
                .iter()
                .find(|(id, _)| *id == local_player_id)
                .map(|(_, e)| *e);
            let movement = if let Some(entity) = local_entity {
                if let Some(pi) = world.get::<PlayerInput>(entity).cloned() {
                    compute_movement(&keys, &pi)
                } else {
                    compute_movement_default(&keys)
                }
            } else {
                compute_movement_default(&keys)
            };
            (movement.0, movement.1, 0, None)
        };
        let input = InputFrame {
            tick: runtime.session.current_tick(),
            player_id: runtime.session.local_peer_id(),
            move_x,
            move_y,
            action_bits,
            ctf_pointer,
        };

        runtime.session.enqueue_local_input(input.clone());
        let mut ctf_frames = Vec::new();
        if game_mode == GameMode::CaptureTheFlag {
            ctf_frames.push(input);
        } else {
            // Apply local prediction immediately (dead reckoning).
            apply_player_velocity(world, player_entities, local_player_id, move_x, move_y);
        }

        runtime.session.tick();
        let current_tick = runtime.session.current_tick();
        let events = runtime.session.drain_network_events();

        // Populate NetworkResource so systems can consume events this tick.
        if let Some(net_res) = world.resource_mut::<NetworkResource>() {
            net_res.pending_events = events.clone();
        }

        for event in events {
            match event {
                NetworkEvent::CorrectionReceived { snapshot, .. } => {
                    apply_snapshot(world, &snapshot);
                }
                NetworkEvent::InputReceived(input) => {
                    if game_mode == GameMode::CaptureTheFlag {
                        ctf_frames.push(input);
                    } else {
                        let InputFrame { player_id, move_x, move_y, .. } = input;
                        apply_player_velocity(world, player_entities, player_id, move_x, move_y);
                    }
                }
                NetworkEvent::HashMismatch { .. } => {}
                NetworkEvent::HostHashReceived { tick, host_hash } => {
                    // Client: compare host hash against local state at the same tick.
                    // On mismatch, request a correction by checking with host.
                    let local = state_hash(world, tick);
                    if local != host_hash {
                        println!(
                            "[client] hash mismatch at tick {tick}: local={local:#x} host={host_hash:#x}"
                        );
                        // The host will send a HostCorrection which we'll apply next tick.
                    }
                }
            }
        }

        if game_mode == GameMode::CaptureTheFlag {
            world.insert_resource(CtfInputState { frames: ctf_frames });
        }

        // Host: periodically broadcast state hash for desync detection.
        if runtime.session.is_host() {
            runtime.hash_check_counter += 1;
            let stride = if game_mode == GameMode::CaptureTheFlag {
                DEFAULT_SNAPSHOT_STRIDE
            } else {
                HASH_CHECK_INTERVAL
            };
            if runtime.hash_check_counter >= stride {
                runtime.hash_check_counter = 0;
                let hash = state_hash(world, current_tick);
                runtime.session.broadcast_hash(current_tick, hash);

                // Also send a correction so clients can reconcile immediately.
                let snapshot = capture_snapshot(world, current_tick);
                runtime.session.send_correction(current_tick, snapshot);
            }
        }
    }
}

fn render(s: &mut DemoState) {
    s.core.render_ctx.sync_with_window(s.core.platform.window());

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
        // Only render when a Camera exists — no camera = intentional black screen.
        let camera = s
            .core
            .world
            .query2::<Camera, Transform>()
            .next()
            .map(|(_, cam, tf)| (tf.position.x, tf.position.y, cam.zoom));

        if let Some((cam_x, cam_y, cam_zoom)) = camera {
            s.core.world.query3::<Transform, Shape, Color>().for_each(
                |(_, transform, shape, color)| {
                    let mut cmd = make_draw_cmd(&transform, shape, color);
                    // Apply camera offset and zoom, centering the camera target onscreen.
                    cmd = apply_camera_to_cmd(
                        cmd,
                        cam_x,
                        cam_y,
                        cam_zoom,
                        s.core.render_ctx.surface_config.width as f32,
                        s.core.render_ctx.surface_config.height as f32,
                    );
                    s.core.draw_queue.push(cmd);
                },
            );
        }
        // No camera → draw_queue stays empty → black background.
    }

    s.core.draw_queue.flush(
        &s.core.render_ctx,
        &view,
        &mut encoder,
        &s.core.circle_pipeline,
        &s.core.rect_pipeline,
        &s.core.triangle_pipeline,
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
    } else if s.scene_initialized && s.game_mode == GameMode::CaptureTheFlag {
        let ui = s.core.imgui.begin_frame(s.core.platform.window());
        draw_ctf_hud(ui, &s.core.world);
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
        Shape::Triangle { size } => DrawCommand::Triangle {
            x: transform.position.x,
            y: transform.position.y,
            size: *size,
            rotation: transform.rotation,
            color: [color.r, color.g, color.b, color.a],
        },
    }
}

/// Translate and scale a draw command by the active camera.
fn apply_camera_to_cmd(
    cmd: DrawCommand,
    cam_x: f32,
    cam_y: f32,
    zoom: f32,
    viewport_w: f32,
    viewport_h: f32,
) -> DrawCommand {
    let center_x = viewport_w * 0.5;
    let center_y = viewport_h * 0.5;
    match cmd {
        DrawCommand::Circle { x, y, radius, color } => DrawCommand::Circle {
            x: (x - cam_x) * zoom + center_x,
            y: (y - cam_y) * zoom + center_y,
            radius: radius * zoom,
            color,
        },
        DrawCommand::Rect { x, y, width, height, color } => DrawCommand::Rect {
            x: (x - cam_x) * zoom + center_x,
            y: (y - cam_y) * zoom + center_y,
            width: width * zoom,
            height: height * zoom,
            color,
        },
        DrawCommand::Triangle {
            x,
            y,
            size,
            rotation,
            color,
        } => DrawCommand::Triangle {
            x: (x - cam_x) * zoom + center_x,
            y: (y - cam_y) * zoom + center_y,
            size: size * zoom,
            rotation,
            color,
        },
    }
}

fn bootstrap_session(
    mode: &Mode,
    matchmaker_addr: &str,
    gameplay_port: u16,
) -> io::Result<Option<MatchSession>> {
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
            let game_addr = resolve_or_default_game_addr(game_addr.clone(), addr, gameplay_port)?;
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
                    game_mode: GameMode::DefaultScene,
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
                Some(game_addr),
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
            let game_addr = resolve_or_default_game_addr(game_addr.clone(), addr, gameplay_port)?;
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
                Some(game_addr),
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
                game_mode,
                ctf_assignments,
            }) => {
                if started_code == lobby_code {
                    return Ok(MatchState {
                        lobby_code: started_code,
                        host_peer_id: host_client_id,
                        shared_seed: seed,
                        players: player_endpoints,
                        start_tick: 0,
                        game_mode,
                        ctf_assignments,
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
            gameplay_port: cli.game_port,
        },
        Some(Mode::Single) => StartupMode::Single,
        Some(mode) => match bootstrap_session(&mode, &cli.matchmaker, cli.game_port) {
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
