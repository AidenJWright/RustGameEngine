//! Sinusoid system — oscillates entity Y position using a sine wave.
//!
//! Functional pipeline:
//! 1. Read `ElapsedTime` resource.
//! 2. `query2::<Transform, SinusoidComponent>` — all entities with both.
//! 3. Map each to `(entity, new_transform)` where
//!    `y = base_y + amplitude * sin(frequency * elapsed + phase)`.
//! 4. Queue `CommandBuffer::insert` with the updated transform.

use crate::components::Transform;
use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::component::Component;
use crate::ecs::resource::ElapsedTime;
use crate::ecs::system::System;
use crate::ecs::world::World;
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

/// Applies sinusoidal Y motion to entities that have both
/// [`Transform`] and [`SinusoidComponent`].
#[derive(Debug, Default)]
pub struct SinusoidSystem;

impl System for SinusoidSystem {
    fn run(&self, world: &World, commands: &mut CommandBuffer) {
        // Step 1: read total elapsed time (seconds since engine start).
        let elapsed = world
            .resource::<ElapsedTime>()
            .copied()
            .unwrap_or_default()
            .0;

        // Step 2–4: compute new Y for every sinusoidal entity.
        world
            .query2::<Transform, SinusoidComponent>()
            .map(|(entity, transform, sinusoid)| {
                // y = base_y + amplitude * sin(freq * t + phase)
                let new_y = sinusoid.base_y
                    + sinusoid.amplitude * (sinusoid.frequency * elapsed + sinusoid.phase).sin();
                let new_transform = Transform {
                    position: crate::math::Vec3::new(
                        transform.position.x,
                        new_y,
                        transform.position.z,
                    ),
                    ..transform.clone()
                };
                (entity, new_transform)
            })
            .for_each(|(entity, t)| commands.insert(entity, t));
    }
}
