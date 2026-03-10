//! Movement system — adds velocity to transform position each frame.
//!
//! Functional pipeline:
//! 1. Read `DeltaTime` resource.
//! 2. `query2::<Transform, Velocity>` to get all entities with both components.
//! 3. For each, compute the new position = old + velocity * dt.
//! 4. Queue `CommandBuffer::insert` with the updated `Transform`.

use crate::components::{Transform, Velocity};
use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::resource::DeltaTime;
use crate::ecs::system::System;
use crate::ecs::world::World;
use crate::math::Vec3;

/// Applies linear velocity to entity positions every frame.
#[derive(Debug, Default)]
pub struct MovementSystem;

impl System for MovementSystem {
    fn run(&self, world: &World, commands: &mut CommandBuffer) {
        // Step 1: fetch delta time (default 0 if not set).
        let dt = world.resource::<DeltaTime>().copied().unwrap_or_default().0;

        // Step 2–4: iterate entities with Transform + Velocity, queue updates.
        world
            .query2::<Transform, Velocity>()
            .map(|(entity, transform, velocity)| {
                // Compute displacement: velocity (dx, dy) scaled by dt.
                let delta = Vec3::new(velocity.dx * dt, velocity.dy * dt, 0.0);
                let new_transform = Transform {
                    position: transform.position + delta,
                    ..transform.clone()
                };
                (entity, new_transform)
            })
            .for_each(|(entity, new_transform)| {
                commands.insert(entity, new_transform);
            });
    }
}
