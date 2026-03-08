//! Health component.

use serde::{Deserialize, Serialize};
use crate::ecs::component::Component;

/// Current and maximum hit-points.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

impl Component for Health {}
