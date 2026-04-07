//! `SpawnPoints` component — marks an entity as a spawn-point registry.
//!
//! Place one entity with this component in your scene.  The game assigns each
//! joining player a slot in order of `client_id`.  If more players try to join
//! than there are spawn positions, additional clients receive a "session full"
//! error from the game launcher.

use serde::{Deserialize, Serialize};

use crate::ecs::component::Component;

/// Holds an ordered list of world-space spawn positions (`[x, y, z]`).
///
/// # Editor workflow
/// 1. Spawn an empty entity in the editor.
/// 2. Add a `SpawnPoints` component via the inspector dropdown.
/// 3. Edit the positions list and save to `scene.json`.
/// 4. The game loads the scene and uses these positions for player spawning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnPoints {
    /// Each entry is a world-space position `[x, y, z]`.
    pub positions: Vec<[f32; 3]>,
}

impl SpawnPoints {
    /// Create a `SpawnPoints` component with the given position list.
    pub fn new(positions: Vec<[f32; 3]>) -> Self {
        Self { positions }
    }

    /// Create a `SpawnPoints` component with two sensible default positions
    /// (left-centre and right-centre, suitable for a 1280×720 viewport).
    pub fn default_two() -> Self {
        Self {
            positions: vec![[-320.0, 0.0, 0.0], [320.0, 0.0, 0.0]],
        }
    }

    /// Number of available spawn slots.
    pub fn count(&self) -> usize {
        self.positions.len()
    }
}

impl Component for SpawnPoints {}
