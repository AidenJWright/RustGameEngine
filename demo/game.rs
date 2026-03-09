//! Game entry point with optional multiplayer host/client runtime.
//!
//! Run commands:
//!   `cargo run --bin game`
//!   `cargo run --bin game -- host Alice 127.0.0.1:7101`
//!   `cargo run --bin game -- join ABC123 Alice 127.0.0.1:7102`

#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::too_many_lines)]

use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use forge_ecs::app::AppCore;
use forge_ecs::components::{Color, Shape, Tag, Transform, Velocity};
use forge_ecs::ecs::resource::{DeltaTime, ElapsedTime};
use forge_ecs::ecs::world::World;
use forge_ecs::messaging::{LoopPhase, MessageBus};
use forge_ecs::multiplayer;
use forge_ecs::multiplayer::matchmaking::{self, MatchEvent, MatchRequest};
use forge_ecs::multiplayer::{apply_snapshot, InputFrame, MatchSession, MatchState, NetworkEvent};
use forge_ecs::platform::{map_window_event, KeyCode, PlatformEvent};
use forge_ecs::renderer::draw::DrawCommand;
use forge_ecs::systems::{SinusoidSystem, MovementSystem};
use forge_ecs::ecs::entity::Entity;
use forge_ecs::math::Vec3;

#[derive(Debug, Parser)]
#[command(name = "game", about = "Host/client demo runtime with optional matchmaking")]
struct Cli {
    /// Matchmaker server address used by host/join mode.
    #[arg(long, default_value = "127.0.0.1:7000")]
    matchmaker: String,

    #[command(subcommand)]
    mode: Option<Mode>,
}

#[derive(Debug, Subcommand)]
enum Mode {
    /// Run single-player locally (default when omitted).
    Single,
    /// Create a lobby and become the host.
    Host {
        /// Display name in match events.
        player_name: String,
        /// Advertised gameplay endpoint for direct gameplay transport.
        game_addr: String,
    },
    /// Join an existing lobby and wait for host start.
    Join {
        /// Lobby code.
        lobby_code: String,
        /// Display name in match events.
        player_name: String,
        /// Advertised gameplay endpoint for direct gameplay transport.
        game_addr: String,
    },
}

#[derive(Debug, Default)]
struct InputState {
    left: bool,
    right: bool,
    up: bool,
    down: bool,
}

impl InputState {
    fn movement_axis(&self) -> (f32, f32) {
        let x =
            (if self.left { -1.0_f32 } else { 0.0_f32 }) + (if self.right { 1.0_f32 } else { 0.0_f32 });
        let y =
            (if self.up { -1.0_f32 } else { 0.0_f32 }) + (if self.down { 1.0_f32 } else { 0.0_f32 });

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
    PlayerColor { r: 1.0, g: 0.2, b: 0.2, a: 1.0 },
    PlayerColor { r: 0.2, g: 0.6, b: 1.0, a: 1.0 },
    PlayerColor { r: 0.3, g: 0.9, b: 0.3, a: 1.0 },
    PlayerColor { r: 1.0, g: 1.0, b: 0.2, a: 1.0 },
];

struct DemoState {
    core: AppCore,
    bus: MessageBus,
    last_time: Instant,
    player_entities: Vec<(u64, Entity)>,
    local_player_id: u64,
    input_state: InputState,
    multiplayer: Option<MultiplayerRuntime>,
}

struct GameApp {
    pending_session: Option<MatchSession>,
    state: Option<DemoState>,
}

impl ApplicationHandler for GameApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() { return; }

        let attrs = WindowAttributes::default()
            .with_title("Forge ECS -- Game")
            .with_inner_size(PhysicalSize::new(1280_u32, 720_u32))
            .with_resizable(true);

        let window: Window = event_loop.create_window(attrs).expect("failed to create window");
        let mut core = AppCore::from_window(window).expect("AppCore creation failed");
        let window_width = core.render_ctx.surface_config.width as f32;
        let window_height = core.render_ctx.surface_config.height as f32;

        let mut bus = MessageBus::new();
        bus.register(LoopPhase::Update, 0, SinusoidSystem);
        bus.register(LoopPhase::Update, 10, MovementSystem);

        let (multiplayer, player_entities, local_player_id, runtime_mode) =
            match self.pending_session.take() {
                Some(session) => {
                    let player_entities =
                        setup_multiplayer_scene(&mut core.world, &session, window_width, window_height);
                    let local_player_id = session.local_peer_id();

                    let role = if session.is_host() { "host" } else { "client" };
                    let info = format!(
                        "runtime mode: multiplayer ({role}, local_peer={}, host_peer={})",
                        session.local_peer_id(),
                        session.host_peer_id()
                    );
                    (
                        Some(MultiplayerRuntime {
                            session,
                            tick_accumulator: 0.0,
                        }),
                        player_entities,
                        local_player_id,
                        info,
                    )
                }
                None => {
                    let (player_entities, local_player_id) =
                        setup_single_player_scene(&mut core.world, window_width, window_height);
                    (None, player_entities, local_player_id, "runtime mode: single".to_string())
                }
            };

        self.state = Some(DemoState {
            core,
            bus,
            last_time: Instant::now(),
            player_entities,
            local_player_id,
            input_state: InputState::default(),
            multiplayer,
        });

        println!("{runtime_mode}");
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        let _ = cause;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(s) = &mut self.state else { return; };
        if window_id != s.core.platform.window.id() { return; }

        match &event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => s.core.render_ctx.resize(size.width, size.height),
            WindowEvent::RedrawRequested => render(s),
            _ => {}
        }

        if let Some(platform_event) = map_window_event(&event) {
            s.input_state.apply(platform_event);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        let Some(s) = &mut self.state else { return; };

        let now = Instant::now();
        let dt = now.duration_since(s.last_time).as_secs_f32();
        s.last_time = now;

        if let Some(r) = s.core.world.resource_mut::<DeltaTime>() { r.0 = dt; }
        if let Some(r) = s.core.world.resource_mut::<ElapsedTime>() { r.0 += dt; }

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
            if *id == s.local_player_id { Some(*entity) } else { None }
        }) {
            apply_local_velocity(&mut s.core.world, entity, &s.input_state);
        }

        s.bus.run_frame(&mut s.core.world);
        s.core.platform.window.request_redraw();
    }
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
    world.insert(circle_entity, Transform {
        position,
        ..Transform::identity()
    });
    world.insert(circle_entity, Shape::Circle { radius: 50.0 });
    world.insert(circle_entity, Color {
        r: color.r,
        g: color.g,
        b: color.b,
        a: color.a,
    });
    world.insert(circle_entity, Velocity { dx: 0.0, dy: 0.0 });

    (vec![(1, circle_entity)], 1)
}

fn setup_multiplayer_scene(world: &mut World, session: &MatchSession, window_width: f32, window_height: f32) -> Vec<(u64, Entity)> {
    let scene_root = world.spawn();
    world.insert(scene_root, Tag::new("scene_root"));
    let player_count = session.players().len().min(4);

    session
        .players()
        .iter()
        .take(player_count)
        .enumerate()
        .map(|(index, player)| {
            let position = spawn_position(index, player_count, window_width, window_height);
            let color = PLAYER_COLORS[index % PLAYER_COLORS.len()];
            let circle_entity = world.spawn_child(scene_root);
            world.insert(circle_entity, Transform {
                position,
                ..Transform::identity()
            });
            world.insert(circle_entity, Shape::Circle { radius: 50.0 });
            world.insert(circle_entity, Color {
                r: color.r,
                g: color.g,
                b: color.b,
                a: color.a,
            });
            world.insert(circle_entity, Velocity { dx: 0.0, dy: 0.0 });
            (player.client_id, circle_entity)
        })
        .collect()
}

fn spawn_position(player_index: usize, total_players: usize, window_width: f32, window_height: f32) -> Vec3 {
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
            let x = if is_left { window_width * 0.25 } else { window_width * 0.75 };
            let y = if is_top { window_height * 0.25 } else { window_height * 0.75 };
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

fn apply_player_velocity(world: &mut World, player_entities: &[(u64, Entity)], player_id: u64, move_x: f32, move_y: f32) {
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
        apply_player_velocity(world, player_entities, local_player_id, move_x, move_y);

        // Transport send/receive and local input staging happen inside `MatchSession::tick`.
        runtime.session.tick();
        runtime.session.drain_network_events().into_iter().for_each(|event| {
            match event {
                NetworkEvent::CorrectionReceived { snapshot, .. } => {
                    apply_snapshot(world, &snapshot);
                }
                NetworkEvent::InputReceived(input) => {
                    let InputFrame { player_id, move_x, move_y, .. } = input;
                    apply_player_velocity(world, player_entities, player_id, move_x, move_y);
                }
                NetworkEvent::HashMismatch { .. } => {}
            }
        });
    }
}

fn render(s: &mut DemoState) {
    let Some((surface_texture, view)) = s.core.render_ctx.begin_frame() else { return; };

    let mut encoder = s.core.render_ctx.device.create_command_encoder(
        &wgpu::CommandEncoderDescriptor { label: Some("game frame") },
    );

    s.core.world.query3::<Transform, Shape, Color>()
        .for_each(|(_, transform, shape, color)| {
            let cmd = make_draw_cmd(&transform, shape, color);
            s.core.draw_queue.push(cmd);
        });

    s.core.draw_queue.flush(
        &s.core.render_ctx,
        &view,
        &mut encoder,
        &s.core.circle_pipeline,
        &s.core.rect_pipeline,
        [0.1, 0.1, 0.1, 1.0],
    );

    s.core.render_ctx.queue.submit(std::iter::once(encoder.finish()));
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
        io::Error::new(io::ErrorKind::InvalidInput, format!("invalid matchmaker address: {error}"))
    })?;

    match mode {
        Mode::Single => Ok(None),
        Mode::Host { player_name, game_addr } => {
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
                },
            )?;

            let (lobby_code, local_player_id) = match create {
                MatchEvent::LobbyCreated { lobby_code, player_id, .. } => (lobby_code, player_id),
                other => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unexpected response while creating lobby: {other:?}"),
                    ));
                }
            };

            println!("host lobby created: code={lobby_code}, player={local_player_id}");
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
        Mode::Join { lobby_code, player_name, game_addr } => {
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
                MatchEvent::LobbyJoined { player_id, lobby_code: _, .. } => player_id,
                other => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unexpected response while joining lobby: {other:?}"),
                    ));
                }
            };

            println!("joined lobby: code={lobby_code}, player={local_player_id}");
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
            return Err(io::Error::new(io::ErrorKind::TimedOut, "match start wait timed out"));
        }

        if is_host && last_start_send.elapsed() > Duration::from_secs(1) {
            let _ = send_no_reply(
                socket,
                matchmaker_addr,
                MatchRequest::StartMatch {
                    lobby_code: lobby_code.to_string(),
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
            Ok(MatchEvent::MatchStart { lobby_code: started_code, host_client_id, seed, player_endpoints }) => {
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
            Ok(MatchEvent::LobbyUpdated { lobby_code: updated_code, lobby }) => {
                if updated_code == lobby_code {
                    println!("lobby {} players={}", updated_code, lobby.players.len());
                }
            }
            Ok(MatchEvent::Error { message }) => {
                println!("matchmaker error: {message}");
            }
            Ok(other) => {
                println!("ignored matchmaker event: {other:?}");
            }
            Err(error) if matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => {}
            Err(error) => return Err(error),
        }
    }
}

fn send_matchmaker_request(socket: &UdpSocket, server: &SocketAddr, request: MatchRequest) -> io::Result<MatchEvent> {
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
    let mode = cli.mode.unwrap_or(Mode::Single);

    let multiplayer = match bootstrap_session(&mode, &cli.matchmaker) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("could not initialize multiplayer mode: {error}");
            std::process::exit(1);
        }
    };

    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut app = GameApp {
        pending_session: multiplayer,
        state: None,
    };
    event_loop.run_app(&mut app).expect("event loop error");
}
