//! Debug system — prints entity/component info to stdout.
//!
//! This is a development aid; disable or remove for release builds.

use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::system::System;
use crate::ecs::world::World;
use crate::components::Transform;

/// Prints each entity's transform to stdout every frame.
///
/// Useful during development; typically only added in debug builds.
#[derive(Debug, Default)]
pub struct DebugSystem;

impl System for DebugSystem {
    fn run(&self, world: &World, _commands: &mut CommandBuffer) {
        world
            .query::<Transform>()
            .for_each(|(entity, transform)| {
                println!(
                    "[DEBUG] {entity} pos=({:.2}, {:.2}, {:.2})",
                    transform.position.x, transform.position.y, transform.position.z
                );
            });
    }
}
