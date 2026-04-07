//! Sinusoid data component — drives sinusoidal Y motion via [`crate::systems::SinusoidSystem`].

use crate::ecs::component::Component;
use serde::{Deserialize, Serialize};

/// Data component that drives sinusoidal Y motion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SinusoidComponent {
    /// Peak displacement from `base_y` in world units.
    pub amplitude: f32,
    /// Oscillations per second.
    pub frequency: f32,
    /// Phase offset in radians.
    pub phase: f32,
    /// The Y position the entity rests at when `sin = 0`.
    pub base_y: f32,
}

impl Component for SinusoidComponent {}
