//! Velocity component — 2D movement per second.

use crate::ecs::component::Component;

/// Linear velocity in world units per second.
#[derive(Debug, Clone)]
pub struct Velocity {
    pub dx: f32,
    pub dy: f32,
}

impl Component for Velocity {}
