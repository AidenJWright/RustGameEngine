//! Shared protocol for matchmaker requests and events.

use std::io;
use std::net::{SocketAddr, UdpSocket};

use serde::{Deserialize, Serialize};

/// Maximum players that can be in a lobby.
pub const MAX_PLAYERS: usize = 4;
/// Minimum players required to start a match (server policy may require more).
pub const MIN_PLAYERS: usize = 1;

/// Messages sent by clients to the matchmaker.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum MatchRequest {
    /// Lightweight connectivity probe used by launcher UI.
    Ping,
    /// Create a new lobby and become its first member.
    CreateLobby {
        /// Player display name.
        player_name: String,
        /// Advertised gameplay address for peer-to-peer setup.
        game_addr: String,
        /// Desired lobby size including host.
        target_players: u8,
    },
    /// Join an existing lobby by code.
    JoinLobby {
        /// Existing lobby code.
        lobby_code: String,
        /// Player display name.
        player_name: String,
        /// Advertised gameplay address for peer-to-peer setup.
        game_addr: String,
    },
    /// Leave lobby explicitly.
    LeaveLobby {
        /// Lobby code.
        lobby_code: String,
        /// Player identifier assigned by the matchmaker.
        client_id: u64,
    },
    /// Request to start the match (optional; also auto-starts at threshold).
    StartMatch {
        /// Lobby code.
        lobby_code: String,
        /// Player identifier assigned by the matchmaker.
        client_id: u64,
    },
    /// Heartbeat for stale-client cleanup.
    Heartbeat {
        /// Lobby code.
        lobby_code: String,
        /// Player identifier assigned by the matchmaker.
        client_id: u64,
        /// Optional refresh of advertised gameplay endpoint.
        game_addr: Option<String>,
    },
}

/// Messages emitted by matchmaker.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum MatchEvent {
    /// Response to a [`MatchRequest::Ping`] check.
    Pong,
    /// New lobby was created.
    LobbyCreated {
        lobby_code: String,
        player_id: u64,
        lobby: LobbyState,
    },
    /// Joined an existing lobby.
    LobbyJoined {
        lobby_code: String,
        player_id: u64,
        lobby: LobbyState,
    },
    /// General lobby update (player list changes).
    LobbyUpdated {
        lobby_code: String,
        lobby: LobbyState,
    },
    /// Host was selected and the game is now started.
    MatchStart {
        lobby_code: String,
        host_client_id: u64,
        seed: u64,
        player_endpoints: Vec<PlayerInfo>,
    },
    /// Error response for invalid request.
    Error { message: String },
}

/// Basic player descriptor that includes logical identity and gameplay endpoint.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PlayerInfo {
    /// Matchmaker-issued stable id.
    pub client_id: u64,
    /// Player display name.
    pub name: String,
    /// Peer gameplay address for P2P bootstrap.
    pub game_addr: String,
}

/// Snapshot of all lobby-relevant state.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LobbyState {
    /// Lobby join code.
    pub lobby_code: String,
    /// Current connected players.
    pub players: Vec<PlayerInfo>,
    /// Whether the lobby has already started.
    pub started: bool,
    /// Host id if already assigned.
    pub host_client_id: Option<u64>,
    /// Desired lobby size including host.
    pub target_players: u8,
    /// Remaining seconds until auto-start once target has been reached.
    pub countdown_seconds: Option<u64>,
}

pub fn serialize_request<T: Serialize>(message: &T) -> io::Result<Vec<u8>> {
    bincode::serialize(message).map_err(io::Error::other)
}

pub fn deserialize_request<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> io::Result<T> {
    bincode::deserialize(bytes).map_err(io::Error::other)
}

pub fn send_match_event<T: Serialize>(
    socket: &UdpSocket,
    addr: &SocketAddr,
    message: &T,
) -> io::Result<()> {
    let bytes = serialize_request(message)?;
    let _ = socket.send_to(&bytes, addr)?;
    Ok(())
}

pub fn receive_match_request<T: for<'de> Deserialize<'de>>(
    socket: &UdpSocket,
    buffer: &mut [u8],
) -> io::Result<(T, SocketAddr)> {
    let (size, from) = socket.recv_from(buffer)?;
    let message = deserialize_request::<T>(&buffer[..size])?;
    Ok((message, from))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_ping_and_pong() {
        let ping = MatchRequest::Ping;
        let ping_bytes = serialize_request(&ping).expect("serialize ping");
        let decoded_ping =
            deserialize_request::<MatchRequest>(&ping_bytes).expect("deserialize ping");
        assert!(matches!(decoded_ping, MatchRequest::Ping));

        let pong = MatchEvent::Pong;
        let pong_bytes = serialize_request(&pong).expect("serialize pong");
        let decoded_pong =
            deserialize_request::<MatchEvent>(&pong_bytes).expect("deserialize pong");
        assert!(matches!(decoded_pong, MatchEvent::Pong));
    }

    #[test]
    fn roundtrip_lobby_state_new_fields() {
        let state = LobbyState {
            lobby_code: "1234".to_string(),
            players: Vec::new(),
            started: false,
            host_client_id: Some(9),
            target_players: 4,
            countdown_seconds: Some(3),
        };
        let bytes = serialize_request(&state).expect("serialize lobby state");
        let decoded = deserialize_request::<LobbyState>(&bytes).expect("deserialize lobby state");
        assert_eq!(decoded.lobby_code, "1234");
        assert_eq!(decoded.target_players, 4);
        assert_eq!(decoded.countdown_seconds, Some(3));
    }
}
