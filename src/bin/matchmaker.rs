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
    deserialize_request, send_match_event, MatchEvent, MatchRequest, MAX_PLAYERS,
    PlayerInfo, LobbyState,
};

const MATCHMAKER_TICK_MS: u64 = 125;
const STALE_CLIENT_SECS: u64 = 45;
const AUTO_START_AFTER_SECS: u64 = 15;

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
    id: u64,
    info: PlayerInfo,
    remote_addr: SocketAddr,
    last_seen: Instant,
}

#[derive(Debug)]
struct Lobby {
    code: String,
    players: HashMap<u64, LobbyPlayer>,
    created_at: Instant,
    last_activity: Instant,
    started: bool,
    /// Shared deterministic seed used for host election.
    shared_seed: u64,
    /// Host is fixed by creator unless no longer present at start.
    host_client_id: Option<u64>,
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

    println!("Matchmaker listening on {}", bind_addr);

    let mut lobbies: HashMap<String, Lobby> = HashMap::new();
    let mut next_client_id: u64 = 1;
    let mut last_cleanup = Instant::now();
    let mut buffer = [0_u8; 65_536];

    loop {
        if let Err(error) = process_tick(
            &socket,
            &mut buffer,
            &mut lobbies,
            &mut next_client_id,
        ) {
            match error.kind() {
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {}
                _ => eprintln!("matchmaker packet error: {error}"),
            }
        }

        maybe_auto_start_lobbies(&socket, &mut lobbies);

        if last_cleanup.elapsed() >= Duration::from_secs(1) {
            remove_stale_players(&mut lobbies, Duration::from_secs(STALE_CLIENT_SECS));
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
        MatchRequest::CreateLobby { player_name, game_addr } => {
            let (lobby_code, player_id, lobby_state) =
                create_lobby(lobbies, next_client_id, remote_addr, player_name, game_addr);
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
        MatchRequest::JoinLobby {
            lobby_code,
            player_name,
            game_addr,
        } => {
            match join_lobby(
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
                    println!(
                        "join-lobby: player_id={player_id} lobby={lobby_code} from={remote_addr}"
                    );
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
                        &MatchEvent::Error { message: error.to_string() },
                    )?;
                    println!("join-lobby failed from {remote_addr}: {error}");
                }
            }
        }
        MatchRequest::LeaveLobby { lobby_code, client_id } => {
            match leave_lobby(lobbies, &lobby_code, client_id) {
                Ok(None) => {
                    println!("leave-lobby: player_id={client_id} lobby={lobby_code}");
                }
                Ok(Some(mut lobby)) => {
                    let lobby_state = lobby_state_for(&mut lobby);
                    broadcast_lobby(
                        socket,
                        &lobby,
                        MatchEvent::LobbyUpdated {
                            lobby_code: lobby_code.clone(),
                            lobby: lobby_state,
                        },
                    )?;
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
            }
            if let Some(lobby) = lobbies.get(&lobby_code) {
                if lobby.players.is_empty() {
                    lobbies.remove(&lobby_code);
                }
            }
        }
        MatchRequest::StartMatch { lobby_code } => {
            if maybe_auto_start(socket, lobbies, &lobby_code) {
                println!("match-start: lobby={lobby_code}");
            }
        }
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
) -> (String, u64, LobbyState) {
    let lobby_code = generate_lobby_code();
    let client_id = *next_client_id;
    *next_client_id += 1;

    let mut players = HashMap::new();
    players.insert(
        client_id,
        LobbyPlayer {
            id: client_id,
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
    let lobby = Lobby {
        code: lobby_code.clone(),
        players,
        created_at: now,
        last_activity: now,
        started: false,
        shared_seed: rand::thread_rng().gen(),
        host_client_id: Some(client_id),
    };

    let state = lobby_state_for(&lobby);
    lobbies.insert(lobby_code.clone(), lobby);

    (
        lobby_code.clone(),
        client_id,
        state,
    )
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
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "lobby is full (max 4 players)",
        ));
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
            id: client_id,
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

    let state = lobby_state_for(lobby);
    Ok((client_id, state))
}

fn leave_lobby<'a>(
    lobbies: &'a mut HashMap<String, Lobby>,
    lobby_code: &str,
    client_id: u64,
) -> io::Result<Option<&'a mut Lobby>> {
    let lobby = lobbies
        .get_mut(lobby_code)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "lobby not found"))?;

    if !lobby.started && lobby.host_client_id == Some(client_id) {
        lobby.host_client_id = None;
    }

    if lobby.players.remove(&client_id).is_none() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("player {client_id} not in lobby {lobby_code}"),
        ));
    }

    lobby.last_activity = Instant::now();
    Ok(Some(lobby))
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

fn maybe_auto_start(socket: &UdpSocket, lobbies: &mut HashMap<String, Lobby>, lobby_code: &str) -> bool {
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

fn should_auto_start_lobby(lobby: &Lobby) -> bool {
    if lobby.started || lobby.players.is_empty() {
        return false;
    }

    let lobby_is_full = lobby.players.len() >= MAX_PLAYERS;
    let timeout_elapsed = lobby.created_at.elapsed() >= Duration::from_secs(AUTO_START_AFTER_SECS);
    lobby_is_full || timeout_elapsed
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

    let event = MatchEvent::MatchStart {
        lobby_code: lobby_code.to_string(),
        host_client_id,
        seed: lobby.shared_seed,
        player_endpoints: endpoints.clone(),
    };

    broadcast_lobby(socket, lobby, event).is_ok()
}

fn remove_stale_players(lobbies: &mut HashMap<String, Lobby>, timeout: Duration) {
    let now = Instant::now();
    let mut stale_codes: Vec<String> = Vec::new();

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

        for stale in stale_ids {
            lobby.players.remove(&stale);
            lobby.last_activity = now;
        }

        if lobby.players.is_empty() {
            stale_codes.push(lobby.code.clone());
        }
    }

    stale_codes.into_iter().for_each(|code| {
        lobbies.remove(&code);
        println!("removed-stale-lobby {code}");
    });
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

fn generate_lobby_code() -> String {
    let mut rng = rand::thread_rng();
    let charset: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut code = String::with_capacity(6);
    for _ in 0..6 {
        let idx = rng.gen_range(0..charset.len());
        code.push(charset[idx] as char);
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_lobby(player_count: usize, created_ago: Duration, started: bool) -> Lobby {
        let now = Instant::now();
        let mut players = HashMap::new();
        for id in 1..=player_count as u64 {
            players.insert(
                id,
                LobbyPlayer {
                    id,
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
            code: "ABC123".to_string(),
            players,
            created_at: now - created_ago,
            last_activity: now,
            started,
            shared_seed: 42,
            host_client_id: Some(1),
        }
    }

    #[test]
    fn does_not_auto_start_with_two_players_before_timeout() {
        let lobby = test_lobby(2, Duration::from_secs(5), false);
        assert!(!should_auto_start_lobby(&lobby));
    }

    #[test]
    fn auto_starts_when_lobby_is_full() {
        let lobby = test_lobby(MAX_PLAYERS, Duration::from_secs(1), false);
        assert!(should_auto_start_lobby(&lobby));
    }

    #[test]
    fn auto_starts_after_timeout_even_if_not_full() {
        let lobby = test_lobby(2, Duration::from_secs(AUTO_START_AFTER_SECS), false);
        assert!(should_auto_start_lobby(&lobby));
    }
}
