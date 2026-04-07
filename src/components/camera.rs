//! Camera component — marks an entity as the active viewport camera.
//!
//! The rendering system uses the `Transform` of the entity carrying this
//! component to determine the centre of the viewport.
//!
//! # Rules
//! - Only **one** Camera may exist in the scene at once (singleton constraint).
//!   Attempting to spawn a second Camera should be rejected at spawn time.
//! - Camera is **not** synchronised over the network so different players can
//!   have independent views of the game.

use crate::ecs::component::Component;
use serde::{Deserialize, Serialize};

/// Marks an entity as the active camera.
///
/// The transform of the entity carrying this component determines the viewport
/// centre in the game renderer.  The component carries a `zoom` multiplier;
/// values above 1.0 bring objects closer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Camera {
    /// Zoom multiplier applied to all world-space draw commands.
    /// `1.0` = no zoom, `2.0` = objects appear twice as large.
    pub zoom: f32,
}

impl Camera {
    /// Create a default camera with 1:1 zoom.
    pub fn new() -> Self {
        Self { zoom: 1.0 }
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for Camera {}
