//! Shared protocol for matchmaker requests and events.

use std::io;
use std::net::{SocketAddr, UdpSocket};

use serde::{Deserialize, Serialize};

/// Maximum players that can be in a lobby.
pub const MAX_PLAYERS: usize = 4;
/// Minimum players required to start a match (server policy may require more).
pub const MIN_PLAYERS: usize = 1;

/// Game mode selected for a lobby.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
pub enum GameMode {
    /// Existing generic scene multiplayer flow.
    #[default]
    DefaultScene,
    /// Team capture-the-flag mode.
    CaptureTheFlag,
}

/// CTF arena size selected for a lobby.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
pub enum MapSize {
    /// 1280 × 720 — original arena (1× area).
    #[default]
    Small,
    /// 2560 × 1440 — 4× area.
    Medium,
    /// 3840 × 2160 — 9× area.
    Large,
}

impl MapSize {
    pub const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Large];

    pub fn label(self) -> &'static str {
        match self {
            Self::Small => "Small  (1280 × 720)",
            Self::Medium => "Medium (2560 × 1440)",
            Self::Large => "Large  (3840 × 2160)",
        }
    }

    pub fn scene_path(self) -> &'static str {
        match self {
            Self::Small => "assets/ctf_arena_small.json",
            Self::Medium => "assets/ctf_arena_medium.json",
            Self::Large => "assets/ctf_arena_large.json",
        }
    }

    /// World-space dimensions (width, height) for this arena.
    pub fn dimensions(self) -> (f32, f32) {
        match self {
            Self::Small => (1280.0, 720.0),
            Self::Medium => (2560.0, 1440.0),
            Self::Large => (3840.0, 2160.0),
        }
    }

    /// X coordinate of the mid-line separating the two teams.
    pub fn midline_x(self) -> f32 {
        self.dimensions().0 * 0.5
    }
}

/// Fixed CTF scene slot.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CtfSlot {
    Red1,
    Red2,
    Red3,
    Red4,
    Blue1,
    Blue2,
    Blue3,
    Blue4,
}

impl CtfSlot {
    pub const COUNT: usize = 8;
    pub const ALL: [Self; Self::COUNT] = [
        Self::Red1,
        Self::Red2,
        Self::Red3,
        Self::Red4,
        Self::Blue1,
        Self::Blue2,
        Self::Blue3,
        Self::Blue4,
    ];
    pub const RED: [Self; 4] = [Self::Red1, Self::Red2, Self::Red3, Self::Red4];
    pub const BLUE: [Self; 4] = [Self::Blue1, Self::Blue2, Self::Blue3, Self::Blue4];
    pub const ASSIGNMENT_SLOTS: [Self; 4] = [Self::Red1, Self::Blue1, Self::Red3, Self::Blue3];

    pub fn label(self) -> &'static str {
        match self {
            Self::Red1 => "Red 1",
            Self::Red2 => "Red 2",
            Self::Red3 => "Red 3",
            Self::Red4 => "Red 4",
            Self::Blue1 => "Blue 1",
            Self::Blue2 => "Blue 2",
            Self::Blue3 => "Blue 3",
            Self::Blue4 => "Blue 4",
        }
    }

    pub fn assignment_label(self) -> &'static str {
        match self {
            Self::Red1 | Self::Red2 => "Red A",
            Self::Red3 | Self::Red4 => "Red B",
            Self::Blue1 | Self::Blue2 => "Blue A",
            Self::Blue3 | Self::Blue4 => "Blue B",
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Self::Red1 => "ctf_player_red_1",
            Self::Red2 => "ctf_player_red_2",
            Self::Red3 => "ctf_player_red_3",
            Self::Red4 => "ctf_player_red_4",
            Self::Blue1 => "ctf_player_blue_1",
            Self::Blue2 => "ctf_player_blue_2",
            Self::Blue3 => "ctf_player_blue_3",
            Self::Blue4 => "ctf_player_blue_4",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Red1 => 0,
            Self::Red2 => 1,
            Self::Red3 => 2,
            Self::Red4 => 3,
            Self::Blue1 => 4,
            Self::Blue2 => 5,
            Self::Blue3 => 6,
            Self::Blue4 => 7,
        }
    }

    pub fn team_number(self) -> u8 {
        match self {
            Self::Red1 | Self::Blue1 => 1,
            Self::Red2 | Self::Blue2 => 2,
            Self::Red3 | Self::Blue3 => 3,
            Self::Red4 | Self::Blue4 => 4,
        }
    }

    pub fn from_team_number(is_red: bool, number: u8) -> Option<Self> {
        match (is_red, number) {
            (true, 1) => Some(Self::Red1),
            (true, 2) => Some(Self::Red2),
            (true, 3) => Some(Self::Red3),
            (true, 4) => Some(Self::Red4),
            (false, 1) => Some(Self::Blue1),
            (false, 2) => Some(Self::Blue2),
            (false, 3) => Some(Self::Blue3),
            (false, 4) => Some(Self::Blue4),
            _ => None,
        }
    }

    pub fn is_red(self) -> bool {
        matches!(self, Self::Red1 | Self::Red2 | Self::Red3 | Self::Red4)
    }

    pub fn is_assignment_slot(self) -> bool {
        matches!(self, Self::Red1 | Self::Red3 | Self::Blue1 | Self::Blue3)
    }

    pub fn team_id(self) -> u8 {
        if self.is_red() {
            1
        } else {
            2
        }
    }

    pub fn teammate_slots(self) -> &'static [Self; 4] {
        if self.is_red() {
            &Self::RED
        } else {
            &Self::BLUE
        }
    }

    pub fn control_pair(self) -> [Self; 2] {
        match self {
            Self::Red1 | Self::Red2 => [Self::Red1, Self::Red2],
            Self::Red3 | Self::Red4 => [Self::Red3, Self::Red4],
            Self::Blue1 | Self::Blue2 => [Self::Blue1, Self::Blue2],
            Self::Blue3 | Self::Blue4 => [Self::Blue3, Self::Blue4],
        }
    }

    pub fn opponent_slots(self) -> &'static [Self; 4] {
        if self.is_red() {
            &Self::BLUE
        } else {
            &Self::RED
        }
    }
}

/// Host-selected primary CTF slot for one lobby player.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct CtfSlotAssignment {
    pub client_id: u64,
    pub primary_slot: CtfSlot,
}

/// Messages sent by clients to the matchmaker.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum MatchRequest {
    /// Lightweight connectivity probe used by launcher UI.
    Ping,
    /// Create a new lobby and become its first member.
    CreateLobby {
        /// Player display name.
        player_name: String,
        /// Desired lobby size including host.
        target_players: u8,
        /// Selected game mode.
        game_mode: GameMode,
        /// CTF arena size (ignored for non-CTF modes).
        map_size: MapSize,
    },
    /// Join an existing lobby by code.
    JoinLobby {
        /// Existing lobby code.
        lobby_code: String,
        /// Player display name.
        player_name: String,
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
    /// Host-only CTF slot assignment update.
    UpdateCtfAssignments {
        /// Lobby code.
        lobby_code: String,
        /// Player identifier assigned by the matchmaker.
        client_id: u64,
        /// Desired CTF slot assignment for current lobby players.
        assignments: Vec<CtfSlotAssignment>,
    },
    /// Heartbeat for stale-client cleanup.
    Heartbeat {
        /// Lobby code.
        lobby_code: String,
        /// Player identifier assigned by the matchmaker.
        client_id: u64,
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
        players: Vec<PlayerInfo>,
        relay: RelayConnectInfo,
        game_mode: GameMode,
        ctf_assignments: Vec<CtfSlotAssignment>,
        map_size: MapSize,
    },
    /// Error response for invalid request.
    Error { message: String },
}

/// Basic player descriptor shared with lobby and gameplay setup.
///
/// This deliberately excludes client network addresses. Once a match starts,
/// clients only receive the relay endpoint owned by the matchmaker process.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PlayerInfo {
    /// Matchmaker-issued stable id.
    pub client_id: u64,
    /// Player display name.
    pub name: String,
}

/// Per-player relay connection details emitted at match start.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RelayConnectInfo {
    /// Stable match/relay identifier. Currently the lobby code.
    pub match_id: String,
    /// Public UDP endpoint for gameplay relay packets.
    pub udp_endpoint: String,
    /// Player id this token authenticates.
    pub client_id: u64,
    /// Short-lived bearer secret used by the relay to bind packets to a player.
    pub session_token: String,
    /// Recommended heartbeat cadence in seconds.
    pub heartbeat_secs: u64,
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
    /// Selected game mode.
    pub game_mode: GameMode,
    /// CTF arena size selected by the host.
    pub map_size: MapSize,
    /// Host-selected CTF slots. Empty for non-CTF lobbies.
    pub ctf_assignments: Vec<CtfSlotAssignment>,
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
            game_mode: GameMode::CaptureTheFlag,
            map_size: MapSize::Medium,
            ctf_assignments: vec![CtfSlotAssignment {
                client_id: 9,
                primary_slot: CtfSlot::Red1,
            }],
        };
        let bytes = serialize_request(&state).expect("serialize lobby state");
        let decoded = deserialize_request::<LobbyState>(&bytes).expect("deserialize lobby state");
        assert_eq!(decoded.lobby_code, "1234");
        assert_eq!(decoded.target_players, 4);
        assert_eq!(decoded.countdown_seconds, Some(3));
        assert_eq!(decoded.game_mode, GameMode::CaptureTheFlag);
        assert_eq!(decoded.map_size, MapSize::Medium);
        assert_eq!(decoded.ctf_assignments[0].primary_slot, CtfSlot::Red1);
    }
}
