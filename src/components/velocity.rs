//! Velocity component — 2D movement per second.

use serde::{Deserialize, Serialize};
use crate::ecs::component::Component;

/// Linear velocity in world units per second.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Velocity {
    pub dx: f32,
    pub dy: f32,
}

impl Component for Velocity {}
