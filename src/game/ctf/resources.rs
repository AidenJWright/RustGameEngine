//! Runtime resources for capture the flag.

use crate::ecs::entity::Entity;
use crate::multiplayer::matchmaking::{CtfSlot, CtfSlotAssignment, MapSize};

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
    pub players: [Entity; CtfSlot::COUNT],
    pub player_spawns: [(f32, f32); CtfSlot::COUNT],
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CarrierState {
    /// Blue-team slot carrying the red flag.
    pub red_flag_carrier: Option<CtfSlot>,
    /// Red-team slot carrying the blue flag.
    pub blue_flag_carrier: Option<CtfSlot>,
}

/// Runtime motion for a flag after it has been thrown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlagMotion {
    pub dir_x: f32,
    pub dir_y: f32,
    pub remaining_distance: f32,
}

/// Active throw motion for each team flag.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FlagMotionState {
    pub red: Option<FlagMotion>,
    pub blue: Option<FlagMotion>,
}

/// World-space waypoint path for MOBA-style auto movement.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoMovePath {
    pub waypoints: Vec<(f32, f32)>,
    pub next_index: usize,
}

/// Active auto-move paths indexed by `CtfSlot::index`.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoMoveState {
    pub paths: [Option<AutoMovePath>; CtfSlot::COUNT],
}

impl Default for AutoMoveState {
    fn default() -> Self {
        Self {
            paths: std::array::from_fn(|_| None),
        }
    }
}

/// Latest pointer state captured by the CTF runner.
#[derive(Debug, Clone, Copy, Default)]
pub struct CtfPointerState {
    pub cursor_world: Option<(f32, f32)>,
    pub pending_left_click_world: Option<(f32, f32)>,
}

/// Selected map size and pending level reload for the post-win restart panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CtfRestartState {
    pub selected_map_size: MapSize,
    pub pending_reload: Option<MapSize>,
}

impl Default for CtfRestartState {
    fn default() -> Self {
        Self {
            selected_map_size: MapSize::Small,
            pending_reload: None,
        }
    }
}

/// Grid used by point-and-click movement.
#[derive(Debug, Clone, PartialEq)]
pub struct NavigationGrid {
    pub cell_size: f32,
    pub cols: usize,
    pub rows: usize,
    pub walkable: Vec<bool>,
    pub arena_width: f32,
    pub arena_height: f32,
}

impl NavigationGrid {
    pub fn is_walkable_index(&self, index: usize) -> bool {
        self.walkable.get(index).copied().unwrap_or(false)
    }
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
    pub restart_down: bool,
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
