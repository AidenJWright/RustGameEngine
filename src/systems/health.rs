//! Health system — despawns entities whose HP has dropped to zero or below.
//!
//! Functional pipeline:
//! 1. `query::<Health>` to find all entities with a Health component.
//! 2. `filter` to keep only those with `current <= 0.0`.
//! 3. `for_each` to queue a `CommandBuffer::despawn` for each.

use crate::components::Health;
use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::system::System;
use crate::ecs::world::World;

/// Despawns any entity whose `Health::current` is ≤ 0.
#[derive(Debug, Default)]
pub struct HealthSystem;

impl System for HealthSystem {
    fn run(&self, world: &World, commands: &mut CommandBuffer) {
        // Step 1: query all Health components.
        // Step 2: keep only dead entities.
        // Step 3: queue despawn for each.
        world
            .query::<Health>()
            .filter(|(_, h)| h.current <= 0.0)
            .map(|(entity, _)| entity)
            .for_each(|entity| commands.despawn(entity));
    }
}
