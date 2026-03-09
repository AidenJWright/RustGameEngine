//! Standalone Renet-targeted matchmaker control plane.
//!
//! Uses UDP request/response messages from [`forge_ecs::multiplayer::matchmaking`]
//! to create/join lobbies, broadcast lobby state, and assign a deterministic host.

use std::collections::HashMap;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use clap::Parser;
use rand::Rng;

use forge_ecs::multiplayer::matchmaking::{
    deserialize_request, send_match_event, LobbyState, MatchEvent, MatchRequest, PlayerInfo,
    MAX_PLAYERS,
};

const MATCHMAKER_TICK_MS: u64 = 125;
const STALE_CLIENT_SECS: u64 = 45;
const AUTO_START_AFTER_TARGET_SECS: u64 = 5;

#[derive(Debug, Parser)]
#[command(
    name = "matchmaker",
    about = "Matchmaker server for Forge ECS multiplayer lobby coordination"
)]
struct Args {
    /// Address that the matchmaker binds to, e.g. 127.0.0.1:7000
    #[arg(long, default_value = "127.0.0.1:7000")]
    bind: String,
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
    /// Instant when required player threshold was reached.
    target_reached_at: Option<Instant>,
    /// Last countdown value broadcast to clients.
    last_countdown_sent: Option<u64>,
}

fn main() {
    let args = Args::parse();
    let bind_addr: SocketAddr = args
        .bind
        .parse()
        .expect("matchmaker --bind must be a valid socket address");

    let socket = UdpSocket::bind(bind_addr).expect("failed to bind matchmaker socket");
    socket
        .set_read_timeout(Some(Duration::from_millis(MATCHMAKER_TICK_MS)))
        .expect("failed to set matchmaker read timeout");

    println!("Matchmaker listening on {bind_addr}");

    let mut lobbies: HashMap<String, Lobby> = HashMap::new();
    let mut next_client_id: u64 = 1;
    let mut last_cleanup = Instant::now();
    let mut buffer = [0_u8; 65_536];

    loop {
        if let Err(error) = process_tick(&socket, &mut buffer, &mut lobbies, &mut next_client_id) {
            match error.kind() {
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {}
                _ => eprintln!("matchmaker packet error: {error}"),
            }
        }

        maybe_auto_start_lobbies(&socket, &mut lobbies);
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
            last_cleanup = Instant::now();
        }
    }
}

fn process_tick(
    socket: &UdpSocket,
    buffer: &mut [u8],
    lobbies: &mut HashMap<String, Lobby>,
    next_client_id: &mut u64,
) -> io::Result<()> {
    let (request, remote_addr) = {
        let (size, from_addr) = socket.recv_from(buffer)?;
        let request = deserialize_request::<MatchRequest>(&buffer[..size])?;
        (request, from_addr)
    };

    match request {
        MatchRequest::Ping => {
            send_match_event(socket, &remote_addr, &MatchEvent::Pong)?;
        }
        MatchRequest::CreateLobby {
            player_name,
            game_addr,
            target_players,
        } => {
            match create_lobby(
                lobbies,
                next_client_id,
                remote_addr,
                player_name,
                game_addr,
                target_players,
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
            game_addr,
        } => match join_lobby(
            lobbies,
            &lobby_code,
            remote_addr,
            player_name,
            game_addr,
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
                maybe_auto_start(socket, lobbies, &lobby_code);
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
        } => match try_start_match_by_requester(socket, lobbies, &lobby_code, client_id) {
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
        MatchRequest::Heartbeat {
            lobby_code,
            client_id,
            game_addr,
        } => {
            heartbeat_lobby(lobbies, &lobby_code, client_id, game_addr, remote_addr)?;
        }
    }

    Ok(())
}

fn create_lobby(
    lobbies: &mut HashMap<String, Lobby>,
    next_client_id: &mut u64,
    remote_addr: SocketAddr,
    player_name: String,
    game_addr: String,
    target_players: u8,
) -> io::Result<(String, u64, LobbyState)> {
    let target_players = validate_target_players(target_players)?;

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
                game_addr,
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
    game_addr: String,
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
                game_addr,
            },
            remote_addr,
            last_seen: now,
        },
    );
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

fn heartbeat_lobby(
    lobbies: &mut HashMap<String, Lobby>,
    lobby_code: &str,
    client_id: u64,
    game_addr: Option<String>,
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
    if let Some(game_addr) = game_addr {
        player.info.game_addr = game_addr;
    }
    lobby.last_activity = now;

    Ok(())
}

fn maybe_auto_start(
    socket: &UdpSocket,
    lobbies: &mut HashMap<String, Lobby>,
    lobby_code: &str,
) -> bool {
    let should_start = lobbies
        .get(lobby_code)
        .map(should_auto_start_lobby)
        .unwrap_or(false);
    if !should_start {
        return false;
    }
    try_start_match(socket, lobbies, lobby_code)
}

fn maybe_auto_start_lobbies(socket: &UdpSocket, lobbies: &mut HashMap<String, Lobby>) {
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
        if try_start_match(socket, lobbies, &code) {
            println!("match-start: lobby={code}");
        }
    }
}

fn try_start_match_by_requester(
    socket: &UdpSocket,
    lobbies: &mut HashMap<String, Lobby>,
    lobby_code: &str,
    requester_client_id: u64,
) -> io::Result<bool> {
    {
        let lobby = lobbies
            .get(lobby_code)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "lobby not found"))?;
        ensure_host_request(lobby, requester_client_id)?;
    }

    Ok(try_start_match(socket, lobbies, lobby_code))
}

fn should_auto_start_lobby(lobby: &Lobby) -> bool {
    if lobby.started || lobby.players.is_empty() {
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
    lobby_code: &str,
) -> bool {
    let lobby = match lobbies.get_mut(lobby_code) {
        Some(lobby) => lobby,
        None => return false,
    };

    if lobby.started || lobby.players.is_empty() {
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

    let mut endpoints = lobby_state_for(lobby).players;
    endpoints.sort_by_key(|player| player.client_id);

    lobby.started = true;
    lobby.host_client_id = Some(host_client_id);
    lobby.last_activity = Instant::now();
    lobby.last_countdown_sent = None;

    let event = MatchEvent::MatchStart {
        lobby_code: lobby_code.to_string(),
        host_client_id,
        seed: lobby.shared_seed,
        player_endpoints: endpoints,
    };

    broadcast_lobby(socket, lobby, event).is_ok()
}

fn remove_stale_players(lobbies: &mut HashMap<String, Lobby>, timeout: Duration) -> Vec<String> {
    let now = Instant::now();
    let mut updated_codes: Vec<String> = Vec::new();
    let mut empty_codes: Vec<String> = Vec::new();

    for lobby in lobbies.values_mut() {
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
    }
}

fn validate_target_players(target_players: u8) -> io::Result<usize> {
    let target = target_players as usize;
    if !(1..=MAX_PLAYERS).contains(&target) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("target_players must be 1..={MAX_PLAYERS}"),
        ));
    }
    Ok(target)
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
                        game_addr: format!("127.0.0.1:{}", 7000 + id),
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
            target_reached_at: reached_ago.map(|duration| now - duration),
            last_countdown_sent: None,
        }
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
}
