//! Shared multiplayer protocol and serialization types.
//!
//! The module intentionally keeps payloads small and deterministic so they can be
//! used both by Renet messages and offline/state-capture tooling.

use serde::{Deserialize, Serialize};

use crate::components::Transform;
use crate::multiplayer::matchmaking::{CtfSlot, CtfSlotAssignment, GameMode};

/// A transport-agnostic network tick index.
pub type NetworkTick = u32;

/// Stable entity identifier suitable for wire payloads.
pub type NetworkEntityId = (u32, u32);

/// Fixed transport channels used by gameplay packets.
pub const NET_CHANNEL_INPUT: u8 = 0;
pub const NET_CHANNEL_CONTROL: u8 = 1;
pub const NET_CHANNEL_CORRECTION: u8 = 2;

/// Default tick-rate used by host/client sessions.
pub const DEFAULT_TICK_RATE: u32 = 60;

/// Default cadence for host snapshots (every N host ticks).
pub const DEFAULT_SNAPSHOT_STRIDE: u32 = 5;

/// Lightweight local input captured for deterministic replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputFrame {
    /// Simulation tick this command belongs to.
    pub tick: NetworkTick,
    /// Logical peer id that produced the input.
    pub player_id: u64,
    /// Horizontal intent, where 1.0 means full positive X input.
    pub move_x: f32,
    /// Vertical intent, where 1.0 means full positive Y input.
    pub move_y: f32,
    /// Additional action bits (jump, shoot, interact, ...).
    pub action_bits: u8,
}

/// Canonical per-entity snapshot row used for host corrections.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityStatePacket {
    /// Network-stable entity identity.
    pub entity: NetworkEntityId,
    /// Position vector used by core transform state.
    pub position: (f32, f32, f32),
    /// Rotation component for future extension.
    pub rotation: f32,
    /// Scale component for future extension.
    pub scale: (f32, f32, f32),
}

impl From<(crate::ecs::entity::Entity, &Transform)> for EntityStatePacket {
    fn from((entity, transform): (crate::ecs::entity::Entity, &Transform)) -> Self {
        Self {
            entity: (entity.index, entity.generation),
            position: (
                transform.position.x,
                transform.position.y,
                transform.position.z,
            ),
            rotation: transform.rotation,
            scale: (transform.scale.x, transform.scale.y, transform.scale.z),
        }
    }
}

/// Full deterministic snapshot used for desync correction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Simulation tick the snapshot corresponds to.
    pub tick: NetworkTick,
    /// Snapshot payload.
    pub entities: Vec<EntityStatePacket>,
    /// Optional CTF-specific authoritative state.
    #[serde(default)]
    pub ctf: Option<CtfSnapshotState>,
}

/// CTF state that is not represented by entity transforms.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CtfSnapshotState {
    /// `None` while playing, otherwise winning team id (`1` red, `2` blue).
    pub winner: Option<u8>,
    /// Slot carrying the red flag, if any.
    pub red_flag_carrier: Option<CtfSlot>,
    /// Slot carrying the blue flag, if any.
    pub blue_flag_carrier: Option<CtfSlot>,
    /// Current selected controlled slot per peer.
    pub selected_slots: Vec<CtfSlotAssignment>,
}

/// Sync/correction protocol over the gameplay transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetMessage {
    /// Local command intent for input-replication.
    Input(InputFrame),
    /// Checkpoint hash used to detect divergence.
    HostHash {
        /// Simulation tick for this hash.
        tick: NetworkTick,
        /// 64-bit hash value of host authoritative state.
        hash: u64,
    },
    /// Full correction snapshot for one tick.
    HostCorrection {
        /// Simulation tick this correction is for.
        tick: NetworkTick,
        /// Authoritative state that should be restored.
        snapshot: Snapshot,
    },
}

/// Internal sync policy consumed by multiplayer sessions.
#[derive(Debug, Clone)]
pub enum SyncMode {
    /// Replicate only local inputs; host remains authoritative.
    InputReplication,
    /// Full lockstep mode (reserved for future extension).
    Lockstep,
}

/// Internal desync policy consumed by multiplayer sessions.
#[derive(Debug, Clone)]
pub enum DesyncMode {
    /// Host sends snapshots only after a detected mismatch.
    SnapshotCorrections,
    /// Always send snapshots every tick.
    AggressiveSnapshots,
}

/// Combined network policy for session behavior.
#[derive(Debug, Clone)]
pub struct NetworkPolicy {
    /// Desired sync style.
    pub sync_mode: SyncMode,
    /// Desired desync strategy.
    pub desync_mode: DesyncMode,
    /// Simulation ticks per second.
    pub tick_rate: u32,
}

/// API name expected by gameplay callers.
pub type MultiplayerConfig = NetworkPolicy;

impl Default for NetworkPolicy {
    fn default() -> Self {
        Self {
            sync_mode: SyncMode::InputReplication,
            desync_mode: DesyncMode::SnapshotCorrections,
            tick_rate: DEFAULT_TICK_RATE,
        }
    }
}

/// Aggregate match state shared between matchmaker and gameplay layers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchState {
    /// Lobby join code used by the control plane.
    pub lobby_code: String,
    /// Host peer identifier selected after deterministic election.
    pub host_peer_id: u64,
    /// Shared deterministic seed for host election.
    pub shared_seed: u64,
    /// Ordered player list.
    pub players: Vec<crate::multiplayer::matchmaking::PlayerInfo>,
    /// Host tick at which authoritative game state starts.
    pub start_tick: NetworkTick,
    /// Selected game mode for this match.
    pub game_mode: GameMode,
    /// Host-selected CTF slot assignments. Empty for non-CTF matches.
    pub ctf_assignments: Vec<CtfSlotAssignment>,
}

/// Input frame alias kept for API stability with plan documentation.
pub type PlayerInputFrame = InputFrame;

/// Runtime event stream that session consumers can react to.
#[derive(Debug, Clone)]
pub enum NetworkEvent {
    /// Received a remote command.
    InputReceived(InputFrame),
    /// Host authoritative state correction.
    CorrectionReceived {
        /// Simulation tick to reconcile onto.
        tick: NetworkTick,
        /// Authoritative full snapshot.
        snapshot: Snapshot,
    },
    /// Mismatch between local host hash and remote peer hash.
    HashMismatch {
        /// Simulation tick for the mismatch.
        tick: NetworkTick,
        /// Local hash value.
        local_hash: u64,
        /// Authoritative hash value.
        remote_hash: u64,
    },
    /// Host broadcast its authoritative state hash for validation.
    ///
    /// The game loop should compare this against the local `state_hash`
    /// and trigger a correction if they differ.
    HostHashReceived {
        /// Simulation tick for this hash checkpoint.
        tick: NetworkTick,
        /// Authoritative hash value from the host.
        host_hash: u64,
    },
}

/// ECS resource that decouples the game loop from the `MatchSession`.
///
/// Each frame the network tick loop drains session events into this resource
/// *before* calling `bus.run_frame()`.  Game systems read from here instead of
/// touching `MatchSession` directly, keeping networking out of the system layer.
///
/// After `bus.run_frame()` the loop drains any outbound input from this resource
/// and sends it via the session.
pub struct NetworkResource {
    /// Events that arrived from the network this frame.
    pub pending_events: Vec<NetworkEvent>,
    /// Peer ID of the local player.
    pub local_peer_id: u64,
    /// `true` when this peer is the authoritative host.
    pub is_host: bool,
    /// Outbound input queued by game logic for the session to send.
    pub outbound_input: Option<InputFrame>,
}
