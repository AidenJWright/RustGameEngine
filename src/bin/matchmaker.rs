//! Standalone Renet-targeted matchmaker control plane.
//!
//! Uses UDP request/response messages from [`forge_ecs::multiplayer::matchmaking`]
//! to create/join lobbies, broadcast lobby state, and assign a deterministic host.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;
use rand::Rng;

use forge_ecs::multiplayer::matchmaking::{
    deserialize_request, send_match_event, CtfSlot, CtfSlotAssignment, GameMode, LobbyState,
    MapSize, MatchEvent, MatchRequest, PlayerInfo, RelayConnectInfo, MAX_PLAYERS,
};
use forge_ecs::multiplayer::net_types::{
    deserialize_relay_packet, serialize_relay_packet, NetMessage, RelayClientPacket,
    RelayServerPacket, RELAY_PROTOCOL_VERSION,
};

const STALE_CLIENT_SECS: u64 = 45;
const STALE_RELAY_CLIENT_SECS: u64 = 60;
const AUTO_START_AFTER_TARGET_SECS: u64 = 5;
const DEFAULT_RELAY_HEARTBEAT_SECS: u64 = 5;

#[derive(Debug, Parser)]
#[command(
    name = "matchmaker",
    about = "Matchmaker server for Forge ECS multiplayer lobby coordination"
)]
struct Args {
    /// Address that the matchmaker binds to.
    ///
    /// For local testing: `127.0.0.1:7000` (default).
    /// For remote server deployment: `0.0.0.0:7000` to accept connections on all interfaces.
    /// Override with the `MATCHMAKER_BIND` environment variable instead of a flag if preferred.
    #[arg(long, default_value = "127.0.0.1:7000", env = "MATCHMAKER_BIND")]
    bind: String,

    /// Address that the gameplay relay binds to.
    ///
    /// For deployment use `0.0.0.0:7001`. A privileged process can also bind a
    /// well-known UDP port such as `0.0.0.0:443` on a dedicated relay VM.
    #[arg(long, default_value = "127.0.0.1:7001", env = "RELAY_BIND")]
    relay_bind: String,

    /// Public relay endpoint sent to matched clients.
    ///
    /// If omitted, the relay bind address is advertised unless it uses an
    /// unspecified IP, in which case `127.0.0.1:<relay-port>` is used.
    #[arg(long, env = "RELAY_ADVERTISE")]
    relay_advertise: Option<String>,
}

#[derive(Debug)]
struct LobbyPlayer {
    info: PlayerInfo,
    remote_addr: SocketAddr,
    last_seen: Instant,
}

#[derive(Debug)]
struct Lobby {
    code: String,
    players: HashMap<u64, LobbyPlayer>,
    last_activity: Instant,
    started: bool,
    /// Shared deterministic seed used for host election.
    shared_seed: u64,
    /// Host is fixed by creator unless no longer present.
    host_client_id: Option<u64>,
    /// Desired lobby size including host.
    target_players: usize,
    /// Game mode selected when the lobby was created.
    game_mode: GameMode,
    /// CTF arena size selected when the lobby was created.
    map_size: MapSize,
    /// Host-selected CTF assignments.
    ctf_assignments: Vec<CtfSlotAssignment>,
    /// Instant when required player threshold was reached.
    target_reached_at: Option<Instant>,
    /// Last countdown value broadcast to clients.
    last_countdown_sent: Option<u64>,
}

#[derive(Debug)]
struct RelayPlayer {
    token: String,
    remote_addr: Option<SocketAddr>,
    last_seen: Instant,
}

#[derive(Debug)]
struct RelayMatch {
    players: HashMap<u64, RelayPlayer>,
    last_activity: Instant,
}

fn main() {
    let args = Args::parse();
    let bind_addr: SocketAddr = args
        .bind
        .parse()
        .expect("matchmaker --bind must be a valid socket address");
    let relay_bind_addr: SocketAddr = args
        .relay_bind
        .parse()
        .expect("matchmaker --relay-bind must be a valid socket address");
    let relay_advertise = args
        .relay_advertise
        .unwrap_or_else(|| advertise_addr_for(relay_bind_addr));

    let socket = UdpSocket::bind(bind_addr).expect("failed to bind matchmaker socket");
    let relay_socket = UdpSocket::bind(relay_bind_addr).expect("failed to bind relay socket");
    socket
        .set_nonblocking(true)
        .expect("failed to set matchmaker nonblocking mode");
    relay_socket
        .set_nonblocking(true)
        .expect("failed to set relay nonblocking mode");

    println!("Matchmaker listening on {bind_addr}");
    println!("Gameplay relay listening on {relay_bind_addr}, advertising {relay_advertise}");

    let mut lobbies: HashMap<String, Lobby> = HashMap::new();
    let mut relay_matches: HashMap<String, RelayMatch> = HashMap::new();
    let mut next_client_id: u64 = 1;
    let mut last_cleanup = Instant::now();
    let mut buffer = [0_u8; 65_536];
    let mut relay_buffer = [0_u8; 65_536];

    loop {
        let mut did_work = false;

        loop {
            match process_control_packet(
                &socket,
                &mut buffer,
                &mut lobbies,
                &mut relay_matches,
                &relay_advertise,
                &mut next_client_id,
            ) {
                Ok(true) => did_work = true,
                Ok(false) => break,
                Err(error) => {
                    eprintln!("matchmaker packet error: {error}");
                    break;
                }
            }
        }

        loop {
            match process_relay_packet(&relay_socket, &mut relay_buffer, &mut relay_matches) {
                Ok(true) => did_work = true,
                Ok(false) => break,
                Err(error) => {
                    eprintln!("relay packet error: {error}");
                    break;
                }
            }
        }

        maybe_auto_start_lobbies(&socket, &mut lobbies, &mut relay_matches, &relay_advertise);
        broadcast_countdown_updates(&socket, &mut lobbies);

        if last_cleanup.elapsed() >= Duration::from_secs(1) {
            let updated_lobbies =
                remove_stale_players(&mut lobbies, Duration::from_secs(STALE_CLIENT_SECS));
            for code in updated_lobbies {
                if let Some(lobby) = lobbies.get(&code) {
                    let _ = broadcast_lobby(
                        &socket,
                        lobby,
                        MatchEvent::LobbyUpdated {
                            lobby_code: code.clone(),
                            lobby: lobby_state_for(lobby),
                        },
                    );
                }
            }
            remove_stale_relay_clients(
                &mut relay_matches,
                Duration::from_secs(STALE_RELAY_CLIENT_SECS),
            );
            last_cleanup = Instant::now();
        }

        if !did_work {
            thread::sleep(Duration::from_millis(1));
        }
    }
}

fn process_control_packet(
    socket: &UdpSocket,
    buffer: &mut [u8],
    lobbies: &mut HashMap<String, Lobby>,
    relay_matches: &mut HashMap<String, RelayMatch>,
    relay_advertise: &str,
    next_client_id: &mut u64,
) -> io::Result<bool> {
    let (request, remote_addr) = {
        let (size, from_addr) = match socket.recv_from(buffer) {
            Ok(value) => value,
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::TimedOut =>
            {
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        let request = deserialize_request::<MatchRequest>(&buffer[..size])?;
        (request, from_addr)
    };

    match request {
        MatchRequest::Ping => {
            send_match_event(socket, &remote_addr, &MatchEvent::Pong)?;
        }
        MatchRequest::CreateLobby {
            player_name,
            target_players,
            game_mode,
            map_size,
        } => {
            match create_lobby(
                lobbies,
                next_client_id,
                remote_addr,
                player_name,
                target_players,
                game_mode,
                map_size,
            ) {
                Ok((lobby_code, player_id, lobby_state)) => {
                    send_match_event(
                        socket,
                        &remote_addr,
                        &MatchEvent::LobbyCreated {
                            lobby_code: lobby_code.clone(),
                            player_id,
                            lobby: lobby_state,
                        },
                    )?;
                    println!(
                        "create-lobby: player_id={player_id} lobby={lobby_code} from={remote_addr}"
                    );
                    if let Some(lobby) = lobbies.get(&lobby_code) {
                        broadcast_lobby(
                            socket,
                            lobby,
                            MatchEvent::LobbyUpdated {
                                lobby_code,
                                lobby: lobby_state_for(lobby),
                            },
                        )?;
                    }
                }
                Err(error) => {
                    send_match_event(
                        socket,
                        &remote_addr,
                        &MatchEvent::Error {
                            message: error.to_string(),
                        },
                    )?;
                }
            }
        }
        MatchRequest::JoinLobby {
            lobby_code,
            player_name,
        } => match join_lobby(
            lobbies,
            &lobby_code,
            remote_addr,
            player_name,
            *next_client_id,
        ) {
            Ok((player_id, lobby_state)) => {
                *next_client_id += 1;
                send_match_event(
                    socket,
                    &remote_addr,
                    &MatchEvent::LobbyJoined {
                        lobby_code: lobby_code.clone(),
                        player_id,
                        lobby: lobby_state.clone(),
                    },
                )?;
                println!("join-lobby: player_id={player_id} lobby={lobby_code} from={remote_addr}");
                if let Some(lobby) = lobbies.get(&lobby_code) {
                    broadcast_lobby(
                        socket,
                        lobby,
                        MatchEvent::LobbyUpdated {
                            lobby_code: lobby_code.clone(),
                            lobby: lobby_state,
                        },
                    )?;
                }
                maybe_auto_start(socket, lobbies, relay_matches, relay_advertise, &lobby_code);
            }
            Err(error) => {
                send_match_event(
                    socket,
                    &remote_addr,
                    &MatchEvent::Error {
                        message: error.to_string(),
                    },
                )?;
                println!("join-lobby failed from {remote_addr}: {error}");
            }
        },
        MatchRequest::LeaveLobby {
            lobby_code,
            client_id,
        } => match leave_lobby(lobbies, &lobby_code, client_id) {
            Ok(Some(state)) => {
                if let Some(lobby) = lobbies.get(&lobby_code) {
                    broadcast_lobby(
                        socket,
                        lobby,
                        MatchEvent::LobbyUpdated {
                            lobby_code,
                            lobby: state,
                        },
                    )?;
                }
                println!("leave-lobby: player_id={client_id}");
            }
            Ok(None) => {
                println!("leave-lobby: removed empty lobby={lobby_code}");
            }
            Err(error) => {
                send_match_event(
                    socket,
                    &remote_addr,
                    &MatchEvent::Error {
                        message: error.to_string(),
                    },
                )?;
                println!("leave-lobby failed from {remote_addr}: {error}");
            }
        },
        MatchRequest::StartMatch {
            lobby_code,
            client_id,
        } => match try_start_match_by_requester(
            socket,
            lobbies,
            relay_matches,
            relay_advertise,
            &lobby_code,
            client_id,
        ) {
            Ok(true) => {
                println!("match-start: lobby={lobby_code} by host={client_id}");
            }
            Ok(false) => {}
            Err(error) => {
                send_match_event(
                    socket,
                    &remote_addr,
                    &MatchEvent::Error {
                        message: error.to_string(),
                    },
                )?;
            }
        },
        MatchRequest::UpdateCtfAssignments {
            lobby_code,
            client_id,
            assignments,
        } => match update_ctf_assignments(lobbies, &lobby_code, client_id, assignments) {
            Ok(state) => {
                if let Some(lobby) = lobbies.get(&lobby_code) {
                    broadcast_lobby(
                        socket,
                        lobby,
                        MatchEvent::LobbyUpdated {
                            lobby_code,
                            lobby: state,
                        },
                    )?;
                }
            }
            Err(error) => {
                send_match_event(
                    socket,
                    &remote_addr,
                    &MatchEvent::Error {
                        message: error.to_string(),
                    },
                )?;
            }
        },
        MatchRequest::Heartbeat {
            lobby_code,
            client_id,
        } => {
            heartbeat_lobby(lobbies, &lobby_code, client_id, remote_addr)?;
        }
    }

    Ok(true)
}

fn create_lobby(
    lobbies: &mut HashMap<String, Lobby>,
    next_client_id: &mut u64,
    remote_addr: SocketAddr,
    player_name: String,
    target_players: u8,
    game_mode: GameMode,
    map_size: MapSize,
) -> io::Result<(String, u64, LobbyState)> {
    let target_players = validate_target_players(target_players, game_mode)?;

    let lobby_code = generate_unique_lobby_code(lobbies);
    let client_id = *next_client_id;
    *next_client_id += 1;

    let mut players = HashMap::new();
    players.insert(
        client_id,
        LobbyPlayer {
            info: PlayerInfo {
                client_id,
                name: player_name,
            },
            remote_addr,
            last_seen: Instant::now(),
        },
    );

    let now = Instant::now();
    let mut lobby = Lobby {
        code: lobby_code.clone(),
        players,
        last_activity: now,
        started: false,
        shared_seed: rand::thread_rng().gen(),
        host_client_id: Some(client_id),
        target_players,
        game_mode,
        map_size: if game_mode == GameMode::CaptureTheFlag {
            map_size
        } else {
            MapSize::Small
        },
        ctf_assignments: if game_mode == GameMode::CaptureTheFlag {
            vec![CtfSlotAssignment {
                client_id,
                primary_slot: CtfSlot::Red1,
            }]
        } else {
            Vec::new()
        },
        target_reached_at: None,
        last_countdown_sent: None,
    };
    refresh_countdown_state(&mut lobby, now);

    let state = lobby_state_for(&lobby);
    lobbies.insert(lobby_code.clone(), lobby);

    Ok((lobby_code, client_id, state))
}

fn join_lobby(
    lobbies: &mut HashMap<String, Lobby>,
    lobby_code: &str,
    remote_addr: SocketAddr,
    player_name: String,
    next_client_id: u64,
) -> io::Result<(u64, LobbyState)> {
    let now = Instant::now();
    let lobby = lobbies
        .get_mut(lobby_code)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "lobby not found"))?;

    if lobby.players.len() >= MAX_PLAYERS {
        return Err(io::Error::other("lobby is full (max 4 players)"));
    }
    if lobby.started {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "lobby already started",
        ));
    }

    let client_id = next_client_id;
    lobby.players.insert(
        client_id,
        LobbyPlayer {
            info: PlayerInfo {
                client_id,
                name: player_name,
            },
            remote_addr,
            last_seen: now,
        },
    );
    assign_default_ctf_slot(lobby, client_id);
    lobby.last_activity = now;
    refresh_countdown_state(lobby, now);

    let state = lobby_state_for(lobby);
    Ok((client_id, state))
}

fn leave_lobby(
    lobbies: &mut HashMap<String, Lobby>,
    lobby_code: &str,
    client_id: u64,
) -> io::Result<Option<LobbyState>> {
    let now = Instant::now();
    let mut remove_lobby = false;
    let state = {
        let lobby = lobbies
            .get_mut(lobby_code)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "lobby not found"))?;

        if lobby.players.remove(&client_id).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("player {client_id} not in lobby {lobby_code}"),
            ));
        }
        lobby
            .ctf_assignments
            .retain(|assignment| assignment.client_id != client_id);

        reassign_host_if_needed(lobby);
        lobby.last_activity = now;
        refresh_countdown_state(lobby, now);

        if lobby.players.is_empty() {
            remove_lobby = true;
            None
        } else {
            Some(lobby_state_for(lobby))
        }
    };

    if remove_lobby {
        lobbies.remove(lobby_code);
        Ok(None)
    } else {
        Ok(state)
    }
}

fn update_ctf_assignments(
    lobbies: &mut HashMap<String, Lobby>,
    lobby_code: &str,
    requester_client_id: u64,
    assignments: Vec<CtfSlotAssignment>,
) -> io::Result<LobbyState> {
    let lobby = lobbies
        .get_mut(lobby_code)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "lobby not found"))?;
    ensure_host_request(lobby, requester_client_id)?;
    if lobby.game_mode != GameMode::CaptureTheFlag {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "CTF assignments are only valid for capture-the-flag lobbies",
        ));
    }
    if lobby.started {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "lobby already started",
        ));
    }

    validate_ctf_assignments(lobby, &assignments, false)?;
    lobby.ctf_assignments = sorted_assignments(assignments);
    lobby.last_activity = Instant::now();
    Ok(lobby_state_for(lobby))
}

fn heartbeat_lobby(
    lobbies: &mut HashMap<String, Lobby>,
    lobby_code: &str,
    client_id: u64,
    remote_addr: SocketAddr,
) -> io::Result<()> {
    let now = Instant::now();
    let lobby = lobbies
        .get_mut(lobby_code)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "lobby not found"))?;
    let player = lobby
        .players
        .get_mut(&client_id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "player not in lobby"))?;

    player.last_seen = now;
    player.remote_addr = remote_addr;
    lobby.last_activity = now;

    Ok(())
}

fn maybe_auto_start(
    socket: &UdpSocket,
    lobbies: &mut HashMap<String, Lobby>,
    relay_matches: &mut HashMap<String, RelayMatch>,
    relay_advertise: &str,
    lobby_code: &str,
) -> bool {
    let should_start = lobbies
        .get(lobby_code)
        .map(should_auto_start_lobby)
        .unwrap_or(false);
    if !should_start {
        return false;
    }
    try_start_match(socket, lobbies, relay_matches, relay_advertise, lobby_code)
}

fn maybe_auto_start_lobbies(
    socket: &UdpSocket,
    lobbies: &mut HashMap<String, Lobby>,
    relay_matches: &mut HashMap<String, RelayMatch>,
    relay_advertise: &str,
) {
    let to_start: Vec<String> = lobbies
        .iter()
        .filter_map(|(code, lobby)| {
            if should_auto_start_lobby(lobby) {
                Some(code.clone())
            } else {
                None
            }
        })
        .collect();

    for code in to_start {
        if try_start_match(socket, lobbies, relay_matches, relay_advertise, &code) {
            println!("match-start: lobby={code}");
        }
    }
}

fn try_start_match_by_requester(
    socket: &UdpSocket,
    lobbies: &mut HashMap<String, Lobby>,
    relay_matches: &mut HashMap<String, RelayMatch>,
    relay_advertise: &str,
    lobby_code: &str,
    requester_client_id: u64,
) -> io::Result<bool> {
    {
        let lobby = lobbies
            .get(lobby_code)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "lobby not found"))?;
        ensure_host_request(lobby, requester_client_id)?;
        if lobby.game_mode == GameMode::CaptureTheFlag {
            validate_ctf_assignments(lobby, &lobby.ctf_assignments, true)?;
        }
    }

    Ok(try_start_match(
        socket,
        lobbies,
        relay_matches,
        relay_advertise,
        lobby_code,
    ))
}

fn should_auto_start_lobby(lobby: &Lobby) -> bool {
    if lobby.started || lobby.players.is_empty() {
        return false;
    }
    if lobby.game_mode == GameMode::CaptureTheFlag {
        return false;
    }

    let Some(reached_at) = lobby.target_reached_at else {
        return false;
    };

    reached_at.elapsed() >= Duration::from_secs(AUTO_START_AFTER_TARGET_SECS)
}

fn try_start_match(
    socket: &UdpSocket,
    lobbies: &mut HashMap<String, Lobby>,
    relay_matches: &mut HashMap<String, RelayMatch>,
    relay_advertise: &str,
    lobby_code: &str,
) -> bool {
    let lobby = match lobbies.get_mut(lobby_code) {
        Some(lobby) => lobby,
        None => return false,
    };

    if lobby.started || lobby.players.is_empty() {
        return false;
    }
    if lobby.game_mode == GameMode::CaptureTheFlag
        && validate_ctf_assignments(lobby, &lobby.ctf_assignments, true).is_err()
    {
        return false;
    }

    let mut player_ids: Vec<u64> = lobby.players.keys().copied().collect();
    if player_ids.is_empty() {
        return false;
    }

    let host_client_id = match lobby.host_client_id {
        Some(host) if lobby.players.contains_key(&host) => host,
        _ => {
            let host = select_host_client_id(lobby.shared_seed, &mut player_ids);
            lobby.host_client_id = Some(host);
            host
        }
    };

    let mut players = lobby_state_for(lobby).players;
    players.sort_by_key(|player| player.client_id);
    let player_targets: Vec<(u64, SocketAddr)> = lobby
        .players
        .iter()
        .map(|(client_id, player)| (*client_id, player.remote_addr))
        .collect();

    lobby.started = true;
    lobby.host_client_id = Some(host_client_id);
    lobby.last_activity = Instant::now();
    lobby.last_countdown_sent = None;

    let mut relay_players = HashMap::new();
    let mut start_events = Vec::new();

    for (client_id, remote_addr) in player_targets {
        let token = generate_relay_token();
        relay_players.insert(
            client_id,
            RelayPlayer {
                token: token.clone(),
                remote_addr: None,
                last_seen: Instant::now(),
            },
        );
        start_events.push((
            remote_addr,
            MatchEvent::MatchStart {
                lobby_code: lobby_code.to_string(),
                host_client_id,
                seed: lobby.shared_seed,
                players: players.clone(),
                relay: RelayConnectInfo {
                    match_id: lobby_code.to_string(),
                    udp_endpoint: relay_advertise.to_string(),
                    client_id,
                    session_token: token,
                    heartbeat_secs: DEFAULT_RELAY_HEARTBEAT_SECS,
                },
                game_mode: lobby.game_mode,
                map_size: lobby.map_size,
                ctf_assignments: lobby.ctf_assignments.clone(),
            },
        ));
    }

    relay_matches.insert(
        lobby_code.to_string(),
        RelayMatch {
            players: relay_players,
            last_activity: Instant::now(),
        },
    );

    start_events
        .into_iter()
        .all(|(remote_addr, event)| send_match_event(socket, &remote_addr, &event).is_ok())
}

fn process_relay_packet(
    socket: &UdpSocket,
    buffer: &mut [u8],
    relay_matches: &mut HashMap<String, RelayMatch>,
) -> io::Result<bool> {
    let (size, from_addr) = match socket.recv_from(buffer) {
        Ok(value) => value,
        Err(error)
            if error.kind() == io::ErrorKind::WouldBlock
                || error.kind() == io::ErrorKind::TimedOut =>
        {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };

    let packet = deserialize_relay_packet::<RelayClientPacket>(&buffer[..size])?;
    match packet {
        RelayClientPacket::Register {
            protocol_version,
            match_id,
            client_id,
            session_token,
        } => {
            if authenticate_relay_player(
                relay_matches,
                &match_id,
                client_id,
                &session_token,
                from_addr,
                protocol_version,
            )
            .is_ok()
            {
                send_relay_packet(
                    socket,
                    from_addr,
                    &RelayServerPacket::Registered {
                        match_id,
                        client_id,
                    },
                )?;
            } else {
                send_relay_error(socket, from_addr, "relay registration rejected")?;
            }
        }
        RelayClientPacket::Payload {
            protocol_version,
            match_id,
            client_id,
            session_token,
            sequence,
            message,
        } => {
            if let Err(error) = authenticate_relay_player(
                relay_matches,
                &match_id,
                client_id,
                &session_token,
                from_addr,
                protocol_version,
            ) {
                eprintln!("relay auth failed: {error}");
                send_relay_error(socket, from_addr, "relay authentication failed")?;
                return Ok(true);
            }

            relay_gameplay_message(
                socket,
                relay_matches,
                &match_id,
                client_id,
                sequence,
                message,
            )?;
        }
        RelayClientPacket::Heartbeat {
            protocol_version,
            match_id,
            client_id,
            session_token,
        } => {
            if authenticate_relay_player(
                relay_matches,
                &match_id,
                client_id,
                &session_token,
                from_addr,
                protocol_version,
            )
            .is_err()
            {
                send_relay_error(socket, from_addr, "relay heartbeat rejected")?;
            }
        }
    }

    Ok(true)
}

fn authenticate_relay_player(
    relay_matches: &mut HashMap<String, RelayMatch>,
    match_id: &str,
    client_id: u64,
    session_token: &str,
    remote_addr: SocketAddr,
    protocol_version: u16,
) -> io::Result<()> {
    if protocol_version != RELAY_PROTOCOL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported relay protocol version",
        ));
    }

    let relay_match = relay_matches
        .get_mut(match_id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "relay match not found"))?;
    let player = relay_match
        .players
        .get_mut(&client_id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "unknown relay player"))?;

    if player.token != session_token {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "invalid relay token",
        ));
    }

    player.remote_addr = Some(remote_addr);
    player.last_seen = Instant::now();
    relay_match.last_activity = Instant::now();
    Ok(())
}

fn relay_gameplay_message(
    socket: &UdpSocket,
    relay_matches: &HashMap<String, RelayMatch>,
    match_id: &str,
    from_client_id: u64,
    sequence: u64,
    message: NetMessage,
) -> io::Result<()> {
    let relay_match = relay_matches
        .get(match_id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "relay match not found"))?;

    let packet = RelayServerPacket::Payload {
        from_client_id,
        sequence,
        message,
    };
    let payload = serialize_relay_packet(&packet)?;

    for (client_id, player) in &relay_match.players {
        if *client_id == from_client_id {
            continue;
        }
        if let Some(remote_addr) = player.remote_addr {
            let _ = socket.send_to(&payload, remote_addr);
        }
    }

    Ok(())
}

fn send_relay_error(socket: &UdpSocket, remote_addr: SocketAddr, message: &str) -> io::Result<()> {
    send_relay_packet(
        socket,
        remote_addr,
        &RelayServerPacket::Error {
            message: message.to_string(),
        },
    )
}

fn send_relay_packet(
    socket: &UdpSocket,
    remote_addr: SocketAddr,
    packet: &RelayServerPacket,
) -> io::Result<()> {
    let payload = serialize_relay_packet(packet)?;
    let _ = socket.send_to(&payload, remote_addr)?;
    Ok(())
}

fn remove_stale_players(lobbies: &mut HashMap<String, Lobby>, timeout: Duration) -> Vec<String> {
    let now = Instant::now();
    let mut updated_codes: Vec<String> = Vec::new();
    let mut empty_codes: Vec<String> = Vec::new();

    for lobby in lobbies.values_mut() {
        if lobby.started {
            continue;
        }

        let stale_ids: Vec<u64> = lobby
            .players
            .iter()
            .filter_map(|(id, player)| {
                if now.duration_since(player.last_seen) > timeout {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect();

        if stale_ids.is_empty() {
            continue;
        }

        for stale in stale_ids {
            lobby.players.remove(&stale);
            lobby
                .ctf_assignments
                .retain(|assignment| assignment.client_id != stale);
            lobby.last_activity = now;
        }

        reassign_host_if_needed(lobby);
        refresh_countdown_state(lobby, now);

        if lobby.players.is_empty() {
            empty_codes.push(lobby.code.clone());
        } else {
            updated_codes.push(lobby.code.clone());
        }
    }

    for code in empty_codes {
        lobbies.remove(&code);
        println!("removed-stale-lobby {code}");
    }

    updated_codes
}

fn remove_stale_relay_clients(relay_matches: &mut HashMap<String, RelayMatch>, timeout: Duration) {
    let now = Instant::now();
    relay_matches.retain(|match_id, relay_match| {
        relay_match.players.retain(|client_id, player| {
            let keep = now.duration_since(player.last_seen) <= timeout;
            if !keep {
                println!("relay-client-timeout: match={match_id} client={client_id}");
            }
            keep
        });

        let keep_match = !relay_match.players.is_empty();
        if !keep_match {
            println!("relay-match-removed: match={match_id}");
        }
        keep_match
    });
}

fn broadcast_countdown_updates(socket: &UdpSocket, lobbies: &mut HashMap<String, Lobby>) {
    let mut changed_codes: Vec<String> = Vec::new();

    for (code, lobby) in lobbies.iter_mut() {
        if lobby.started || lobby.players.is_empty() {
            continue;
        }

        let remaining = countdown_seconds_remaining(lobby);
        if remaining != lobby.last_countdown_sent {
            lobby.last_countdown_sent = remaining;
            changed_codes.push(code.clone());
        }
    }

    for code in changed_codes {
        if let Some(lobby) = lobbies.get(&code) {
            let _ = broadcast_lobby(
                socket,
                lobby,
                MatchEvent::LobbyUpdated {
                    lobby_code: code.clone(),
                    lobby: lobby_state_for(lobby),
                },
            );
        }
    }
}

fn broadcast_lobby(socket: &UdpSocket, lobby: &Lobby, event: MatchEvent) -> io::Result<()> {
    if lobby.players.is_empty() {
        return Ok(());
    }
    let event_payload = forge_ecs::multiplayer::matchmaking::serialize_request(&event)?;
    lobby.players.values().try_for_each(|player| {
        socket.send_to(&event_payload, player.remote_addr)?;
        Ok::<(), io::Error>(())
    })?;
    Ok(())
}

fn lobby_state_for(lobby: &Lobby) -> LobbyState {
    LobbyState {
        lobby_code: lobby.code.clone(),
        players: lobby
            .players
            .values()
            .map(|player| player.info.clone())
            .collect(),
        started: lobby.started,
        host_client_id: lobby.host_client_id,
        target_players: lobby.target_players as u8,
        countdown_seconds: countdown_seconds_remaining(lobby),
        game_mode: lobby.game_mode,
        map_size: lobby.map_size,
        ctf_assignments: lobby.ctf_assignments.clone(),
    }
}

fn validate_target_players(target_players: u8, game_mode: GameMode) -> io::Result<usize> {
    let target = target_players as usize;
    let min_players = if game_mode == GameMode::CaptureTheFlag {
        2
    } else {
        1
    };
    if !(min_players..=MAX_PLAYERS).contains(&target) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("target_players must be {min_players}..={MAX_PLAYERS}"),
        ));
    }
    Ok(target)
}

fn assign_default_ctf_slot(lobby: &mut Lobby, client_id: u64) {
    if lobby.game_mode != GameMode::CaptureTheFlag {
        return;
    }
    if lobby
        .ctf_assignments
        .iter()
        .any(|assignment| assignment.client_id == client_id)
    {
        return;
    }
    let Some(slot) = CtfSlot::ASSIGNMENT_SLOTS.iter().copied().find(|slot| {
        !lobby
            .ctf_assignments
            .iter()
            .any(|assignment| assignment.primary_slot == *slot)
    }) else {
        return;
    };
    lobby.ctf_assignments.push(CtfSlotAssignment {
        client_id,
        primary_slot: slot,
    });
    lobby.ctf_assignments = sorted_assignments(lobby.ctf_assignments.clone());
}

fn sorted_assignments(mut assignments: Vec<CtfSlotAssignment>) -> Vec<CtfSlotAssignment> {
    assignments.sort_by_key(|assignment| assignment.client_id);
    assignments
}

fn validate_ctf_assignments(
    lobby: &Lobby,
    assignments: &[CtfSlotAssignment],
    require_start_ready: bool,
) -> io::Result<()> {
    if lobby.game_mode != GameMode::CaptureTheFlag {
        return Ok(());
    }
    let player_count = lobby.players.len();
    if require_start_ready && !(2..=MAX_PLAYERS).contains(&player_count) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "CTF requires 2 to 4 players",
        ));
    }
    if assignments.len() != player_count {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "every current lobby player must have exactly one CTF slot",
        ));
    }

    let mut seen_players = std::collections::HashSet::new();
    let mut seen_slots = std::collections::HashSet::new();
    let mut has_red = false;
    let mut has_blue = false;
    let mut red_count = 0_usize;
    let mut blue_count = 0_usize;

    for assignment in assignments {
        if !lobby.players.contains_key(&assignment.client_id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown CTF assignment player {}", assignment.client_id),
            ));
        }
        if !seen_players.insert(assignment.client_id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "duplicate CTF player assignment",
            ));
        }
        if !assignment.primary_slot.is_assignment_slot() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "CTF assignments must choose a two-entity control group",
            ));
        }
        if !seen_slots.insert(assignment.primary_slot) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "duplicate CTF slot assignment",
            ));
        }
        if assignment.primary_slot.is_red() {
            has_red = true;
            red_count += 1;
        } else {
            has_blue = true;
            blue_count += 1;
        }
    }

    if red_count > 2 || blue_count > 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "CTF allows at most two players per team",
        ));
    }

    if require_start_ready && (!has_red || !has_blue) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "CTF requires at least one assigned player per team",
        ));
    }

    Ok(())
}

fn refresh_countdown_state(lobby: &mut Lobby, now: Instant) {
    if lobby.players.len() >= lobby.target_players {
        if lobby.target_reached_at.is_none() {
            lobby.target_reached_at = Some(now);
        }
    } else {
        lobby.target_reached_at = None;
        lobby.last_countdown_sent = None;
    }
}

fn countdown_seconds_remaining(lobby: &Lobby) -> Option<u64> {
    if lobby.started {
        return None;
    }

    let reached_at = lobby.target_reached_at?;
    let elapsed = reached_at.elapsed().as_secs();

    if elapsed >= AUTO_START_AFTER_TARGET_SECS {
        Some(0)
    } else {
        Some(AUTO_START_AFTER_TARGET_SECS - elapsed)
    }
}

fn ensure_host_request(lobby: &Lobby, requester_client_id: u64) -> io::Result<()> {
    match lobby.host_client_id {
        Some(host_id) if host_id == requester_client_id => Ok(()),
        Some(_) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "only the lobby host can start the match",
        )),
        None => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "lobby has no host assigned",
        )),
    }
}

fn reassign_host_if_needed(lobby: &mut Lobby) {
    let needs_new_host = match lobby.host_client_id {
        Some(host_id) => !lobby.players.contains_key(&host_id),
        None => true,
    };

    if needs_new_host {
        lobby.host_client_id = lobby.players.keys().min().copied();
    }
}

fn select_host_client_id(seed: u64, participant_ids: &mut [u64]) -> u64 {
    if participant_ids.is_empty() {
        return 0;
    }

    participant_ids.sort_unstable();

    let mixed = splitmix64(seed ^ (participant_ids.len() as u64).rotate_left(5));
    let index = (mixed as usize) % participant_ids.len();
    participant_ids[index]
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E3779B97F4A7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
    x ^ (x >> 31)
}

fn generate_unique_lobby_code(lobbies: &HashMap<String, Lobby>) -> String {
    let mut attempts = 0_u32;
    loop {
        let code = generate_lobby_code();
        if !lobbies.contains_key(&code) {
            return code;
        }
        attempts += 1;
        if attempts > 20_000 {
            panic!("unable to generate unique 4-digit lobby code");
        }
    }
}

fn generate_lobby_code() -> String {
    let mut rng = rand::thread_rng();
    format!("{:04}", rng.gen_range(0..10_000))
}

fn generate_relay_token() -> String {
    let mut rng = rand::thread_rng();
    format!("{:016x}{:016x}", rng.gen::<u64>(), rng.gen::<u64>())
}

fn advertise_addr_for(bind_addr: SocketAddr) -> String {
    let advertised_ip = if bind_addr.ip().is_unspecified() {
        if bind_addr.is_ipv4() {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else {
            IpAddr::V6(Ipv6Addr::LOCALHOST)
        }
    } else {
        bind_addr.ip()
    };
    SocketAddr::new(advertised_ip, bind_addr.port()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_lobby(
        player_count: usize,
        target_players: usize,
        reached_ago: Option<Duration>,
        started: bool,
    ) -> Lobby {
        let now = Instant::now();
        let mut players = HashMap::new();
        for id in 1..=player_count as u64 {
            players.insert(
                id,
                LobbyPlayer {
                    info: PlayerInfo {
                        client_id: id,
                        name: format!("P{id}"),
                    },
                    remote_addr: format!("127.0.0.1:{}", 9000 + id)
                        .parse()
                        .expect("valid test socket"),
                    last_seen: now,
                },
            );
        }

        Lobby {
            code: "1234".to_string(),
            players,
            last_activity: now,
            started,
            shared_seed: 42,
            host_client_id: Some(1),
            target_players,
            game_mode: GameMode::DefaultScene,
            map_size: MapSize::Small,
            ctf_assignments: Vec::new(),
            target_reached_at: reached_ago.map(|duration| now - duration),
            last_countdown_sent: None,
        }
    }

    fn ctf_test_lobby(player_count: usize, assignments: Vec<CtfSlotAssignment>) -> Lobby {
        let mut lobby = test_lobby(player_count, player_count, None, false);
        lobby.game_mode = GameMode::CaptureTheFlag;
        lobby.ctf_assignments = assignments;
        lobby
    }

    #[test]
    fn lobby_code_is_four_digits() {
        let code = generate_lobby_code();
        assert_eq!(code.len(), 4);
        assert!(code.chars().all(|ch| ch.is_ascii_digit()));
    }

    #[test]
    fn auto_starts_only_after_target_countdown() {
        let not_ready = test_lobby(2, 2, Some(Duration::from_secs(4)), false);
        assert!(!should_auto_start_lobby(&not_ready));

        let ready = test_lobby(
            2,
            2,
            Some(Duration::from_secs(AUTO_START_AFTER_TARGET_SECS)),
            false,
        );
        assert!(should_auto_start_lobby(&ready));
    }

    #[test]
    fn countdown_resets_when_target_not_met() {
        let mut lobby = test_lobby(1, 2, Some(Duration::from_secs(2)), false);
        lobby.last_countdown_sent = Some(3);

        refresh_countdown_state(&mut lobby, Instant::now());

        assert!(lobby.target_reached_at.is_none());
        assert!(lobby.last_countdown_sent.is_none());
        assert_eq!(countdown_seconds_remaining(&lobby), None);
    }

    #[test]
    fn non_host_start_is_rejected() {
        let lobby = test_lobby(2, 2, Some(Duration::from_secs(5)), false);
        let error = ensure_host_request(&lobby, 2).expect_err("non-host should be rejected");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);

        ensure_host_request(&lobby, 1).expect("host should be allowed");
    }

    #[test]
    fn ctf_start_requires_valid_assignments() {
        let missing_blue = ctf_test_lobby(
            2,
            vec![
                CtfSlotAssignment {
                    client_id: 1,
                    primary_slot: CtfSlot::Red1,
                },
                CtfSlotAssignment {
                    client_id: 2,
                    primary_slot: CtfSlot::Red3,
                },
            ],
        );
        assert!(
            validate_ctf_assignments(&missing_blue, &missing_blue.ctf_assignments, true).is_err()
        );

        let duplicate_slot = ctf_test_lobby(
            2,
            vec![
                CtfSlotAssignment {
                    client_id: 1,
                    primary_slot: CtfSlot::Red1,
                },
                CtfSlotAssignment {
                    client_id: 2,
                    primary_slot: CtfSlot::Red1,
                },
            ],
        );
        assert!(
            validate_ctf_assignments(&duplicate_slot, &duplicate_slot.ctf_assignments, true)
                .is_err()
        );

        let valid = ctf_test_lobby(
            2,
            vec![
                CtfSlotAssignment {
                    client_id: 1,
                    primary_slot: CtfSlot::Red1,
                },
                CtfSlotAssignment {
                    client_id: 2,
                    primary_slot: CtfSlot::Blue1,
                },
            ],
        );
        validate_ctf_assignments(&valid, &valid.ctf_assignments, true)
            .expect("valid CTF assignment should pass");
    }

    #[test]
    fn ctf_assignment_update_requires_host() {
        let mut lobbies = HashMap::new();
        let lobby = ctf_test_lobby(
            2,
            vec![
                CtfSlotAssignment {
                    client_id: 1,
                    primary_slot: CtfSlot::Red1,
                },
                CtfSlotAssignment {
                    client_id: 2,
                    primary_slot: CtfSlot::Blue1,
                },
            ],
        );
        lobbies.insert(lobby.code.clone(), lobby);

        let err = update_ctf_assignments(
            &mut lobbies,
            "1234",
            2,
            vec![
                CtfSlotAssignment {
                    client_id: 1,
                    primary_slot: CtfSlot::Red3,
                },
                CtfSlotAssignment {
                    client_id: 2,
                    primary_slot: CtfSlot::Blue3,
                },
            ],
        )
        .expect_err("non-host update should be rejected");
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }
}
