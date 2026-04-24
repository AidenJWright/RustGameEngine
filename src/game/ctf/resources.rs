//! Runtime resources for capture the flag.

use crate::ecs::entity::Entity;
use crate::multiplayer::matchmaking::{CtfSlot, CtfSlotAssignment};

/// High-level CTF match phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamePhase {
    Playing,
    Won(u8),
}

/// Singleton game-state resource.
#[derive(Debug, Clone, Copy)]
pub struct GameState {
    pub phase: GamePhase,
}

impl Default for GameState {
    fn default() -> Self {
        Self {
            phase: GamePhase::Playing,
        }
    }
}

/// Stable entity references and spawn positions resolved during setup.
#[derive(Debug, Clone)]
pub struct EntityRefs {
    pub players: [Entity; 4],
    pub player_spawns: [(f32, f32); 4],
    pub red_flag: Entity,
    pub blue_flag: Entity,
    pub red_flag_spawn: (f32, f32),
    pub blue_flag_spawn: (f32, f32),
}

impl EntityRefs {
    pub fn player(&self, slot: CtfSlot) -> Entity {
        self.players[slot.index()]
    }

    pub fn player_spawn(&self, slot: CtfSlot) -> (f32, f32) {
        self.player_spawns[slot.index()]
    }
}

/// Which player is currently carrying which opponent flag.
#[derive(Debug, Clone, Copy, Default)]
pub struct CarrierState {
    /// Blue-team slot carrying the red flag.
    pub red_flag_carrier: Option<CtfSlot>,
    /// Red-team slot carrying the blue flag.
    pub blue_flag_carrier: Option<CtfSlot>,
}

/// Per-peer ownership and currently selected CTF scene slot.
#[derive(Debug, Clone)]
pub struct PlayerControl {
    pub client_id: u64,
    pub primary_slot: CtfSlot,
    pub selected_slot: CtfSlot,
    pub owned_slots: Vec<CtfSlot>,
    pub tag_down: bool,
    pub switch_down: bool,
}

/// All active CTF control mappings.
#[derive(Debug, Clone, Default)]
pub struct ControlState {
    pub controls: Vec<PlayerControl>,
}

impl ControlState {
    pub fn selected_assignments(&self) -> Vec<CtfSlotAssignment> {
        let mut assignments = self
            .controls
            .iter()
            .map(|control| CtfSlotAssignment {
                client_id: control.client_id,
                primary_slot: control.selected_slot,
            })
            .collect::<Vec<_>>();
        assignments.sort_by_key(|assignment| assignment.client_id);
        assignments
    }
}

/// Input frames gathered for the next CTF simulation pass.
#[derive(Debug, Clone, Default)]
pub struct CtfInputState {
    pub frames: Vec<crate::multiplayer::InputFrame>,
}

/// Marks CTF state as needing an immediate authoritative snapshot.
#[derive(Debug, Clone, Copy, Default)]
pub struct CtfSyncState {
    pub dirty: bool,
}
