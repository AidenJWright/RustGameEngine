//! Health component.

use crate::ecs::component::Component;

/// Current and maximum hit-points.
#[derive(Debug, Clone)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

impl Component for Health {}
