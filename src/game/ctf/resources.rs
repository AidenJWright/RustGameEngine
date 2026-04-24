//! Runtime resources for capture the flag.

use crate::ecs::entity::Entity;

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
#[derive(Debug, Clone, Copy)]
pub struct EntityRefs {
    pub p1: Entity,
    pub p2: Entity,
    pub p1_flag: Entity,
    pub p2_flag: Entity,
    pub p1_spawn: (f32, f32),
    pub p2_spawn: (f32, f32),
    pub p1_flag_spawn: (f32, f32),
    pub p2_flag_spawn: (f32, f32),
}

/// Which player is currently carrying which opponent flag.
#[derive(Debug, Clone, Copy, Default)]
pub struct CarrierState {
    /// Player 1 is carrying Player 2's flag.
    pub p1_carries: bool,
    /// Player 2 is carrying Player 1's flag.
    pub p2_carries: bool,
}

/// Previous-frame tag key state for edge-triggered tagging.
#[derive(Debug, Clone, Copy, Default)]
pub struct TagInputState {
    pub p1_tag_down: bool,
    pub p2_tag_down: bool,
}
