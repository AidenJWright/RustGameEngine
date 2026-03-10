//! RGBA color component — plain data, no methods.

use crate::ecs::component::Component;
use serde::{Deserialize, Serialize};

/// Linear RGBA color with components in `[0.0, 1.0]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Component for Color {}
