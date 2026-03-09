//! Minimal CLI client for the matchmaker control plane.

use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use clap::{Parser, Subcommand};

use forge_ecs::multiplayer::matchmaking::{
    deserialize_request, serialize_request, MatchEvent, MatchRequest,
};

#[derive(Debug, Parser)]
#[command(
    name = "matchmaker_client",
    about = "Command line helper for interacting with the matchmaker"
)]
struct Cli {
    /// Matchmaker address, e.g. 127.0.0.1:7000
    #[arg(long, default_value = "127.0.0.1:7000")]
    server: String,

    /// UDP receive timeout in seconds for request-response commands
    #[arg(long, default_value_t = 2)]
    timeout_secs: u64,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Connectivity check for launcher integration.
    Ping,

    /// Create a lobby and become first member.
    Create {
        player_name: String,
        game_addr: String,
        #[arg(long, default_value_t = 4)]
        target_players: u8,
    },

    /// Join an existing lobby with the provided code.
    Join {
        lobby_code: String,
        player_name: String,
        game_addr: String,
    },

    /// Send a heartbeat so this client does not time out.
    Heartbeat {
        lobby_code: String,
        client_id: u64,
        /// Optional game endpoint to refresh.
        game_addr: Option<String>,
    },

    /// Request that a lobby starts now (optional; auto-start triggers at threshold).
    Start { lobby_code: String, client_id: u64 },

    /// Leave a lobby explicitly.
    Leave { lobby_code: String, client_id: u64 },
}

fn main() {
    let args = Cli::parse();
    let matchmaker_addr: SocketAddr = args
        .server
        .parse()
        .expect("server must be a socket address");

    let socket = UdpSocket::bind("0.0.0.0:0").expect("failed to bind ephemeral UDP socket");

    let (request, expect_reply) = match args.command {
        Command::Ping => (MatchRequest::Ping, true),
        Command::Create {
            player_name,
            game_addr,
            target_players,
        } => (
            MatchRequest::CreateLobby {
                player_name,
                game_addr,
                target_players,
            },
            true,
        ),
        Command::Join {
            lobby_code,
            player_name,
            game_addr,
        } => (
            MatchRequest::JoinLobby {
                lobby_code,
                player_name,
                game_addr,
            },
            true,
        ),
        Command::Heartbeat {
            lobby_code,
            client_id,
            game_addr,
        } => (
            MatchRequest::Heartbeat {
                lobby_code,
                client_id,
                game_addr,
            },
            false,
        ),
        Command::Start {
            lobby_code,
            client_id,
        } => (
            MatchRequest::StartMatch {
                lobby_code,
                client_id,
            },
            false,
        ),
        Command::Leave {
            lobby_code,
            client_id,
        } => (
            MatchRequest::LeaveLobby {
                lobby_code,
                client_id,
            },
            false,
        ),
    };

    let response = send_match_request(
        &socket,
        &matchmaker_addr,
        request,
        expect_reply,
        Duration::from_secs(args.timeout_secs),
    );

    match response {
        Ok(None) => {
            println!("kind=ok");
        }
        Ok(Some(event)) => {
            print_event(&event);
            if matches!(event, MatchEvent::Error { .. }) {
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("matchmaker request failed: {error}");
            std::process::exit(1);
        }
    }
}

fn send_match_request(
    socket: &UdpSocket,
    matchmaker_addr: &SocketAddr,
    request: MatchRequest,
    expect_reply: bool,
    timeout: Duration,
) -> io::Result<Option<MatchEvent>> {
    let request_bytes = serialize_request(&request)?;
    socket.send_to(&request_bytes, matchmaker_addr)?;

    if !expect_reply {
        return Ok(None);
    }

    let mut buffer = [0u8; 65_536];
    socket.set_read_timeout(Some(timeout))?;
    let (size, _) = socket.recv_from(&mut buffer)?;
    let event = deserialize_request::<MatchEvent>(&buffer[..size])?;
    Ok(Some(event))
}

fn print_event(event: &MatchEvent) {
    match event {
        MatchEvent::Pong => {
            println!("kind=pong");
        }
        MatchEvent::LobbyCreated {
            lobby_code,
            player_id,
            lobby,
        } => {
            println!("kind=lobby_created");
            println!("lobby_code={lobby_code}");
            println!("client_id={player_id}");
            println!("lobby_started={}", lobby.started,);
            println!("target_players={}", lobby.target_players);
            println!(
                "countdown_seconds={}",
                lobby.countdown_seconds.map_or(0, |seconds| seconds)
            );
            println!("players={}", lobby.players.len());
        }
        MatchEvent::LobbyJoined {
            lobby_code,
            player_id,
            lobby,
        } => {
            println!("kind=lobby_joined");
            println!("lobby_code={lobby_code}");
            println!("client_id={player_id}");
            println!("lobby_started={}", lobby.started,);
            println!("target_players={}", lobby.target_players);
            println!(
                "countdown_seconds={}",
                lobby.countdown_seconds.map_or(0, |seconds| seconds)
            );
            println!("players={}", lobby.players.len());
        }
        MatchEvent::LobbyUpdated { lobby_code, lobby } => {
            println!("kind=lobby_updated");
            println!("lobby_code={lobby_code}");
            println!("host_client_id={}", lobby.host_client_id.map_or(0, |id| id));
            println!("lobby_started={}", lobby.started,);
            println!("target_players={}", lobby.target_players);
            println!(
                "countdown_seconds={}",
                lobby.countdown_seconds.map_or(0, |seconds| seconds)
            );
            println!("players={}", lobby.players.len());
            for player in &lobby.players {
                println!(
                    "player {}|{}|{}",
                    player.client_id, player.name, player.game_addr
                );
            }
        }
        MatchEvent::MatchStart {
            lobby_code,
            host_client_id,
            seed,
            player_endpoints,
        } => {
            println!("kind=match_start");
            println!("lobby_code={lobby_code}");
            println!("host_client_id={host_client_id}");
            println!("seed={seed}");
            println!("players={}", player_endpoints.len());
            for player in player_endpoints {
                println!(
                    "endpoint {}|{}|{}",
                    player.client_id, player.name, player.game_addr
                );
            }
        }
        MatchEvent::Error { message } => {
            println!("kind=error");
            println!("error={message}");
        }
    }
}
