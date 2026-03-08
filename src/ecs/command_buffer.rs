//! Deferred mutations to [`World`].
//!
//! Systems must not mutate the world while iterating it. Instead they push
//! commands here; the scheduler flushes the buffer after every system run.

use super::component::Component;
use super::entity::Entity;

// ---------------------------------------------------------------------------
// Internal command enum
// ---------------------------------------------------------------------------

/// A single deferred world mutation.
enum Command {
    /// Spawn a new entity (the closure receives it and can insert components).
    Spawn(Box<dyn FnOnce(&mut crate::ecs::world::World)>),
    /// Despawn an entity and all its children.
    Despawn(Entity),
    /// Insert a component on an existing entity.
    InsertComponent {
        #[allow(dead_code)]
        entity: Entity,
        applier: Box<dyn FnOnce(&mut crate::ecs::world::World)>,
    },
    /// Remove a component from an existing entity.
    RemoveComponent {
        #[allow(dead_code)]
        entity: Entity,
        remover: Box<dyn FnOnce(&mut crate::ecs::world::World)>,
    },
}

// ---------------------------------------------------------------------------
// CommandBuffer
// ---------------------------------------------------------------------------

/// Accumulates deferred [`World`] mutations from system runs.
///
/// After each system the scheduler calls [`CommandBuffer::flush`] which
/// replays all commands against the real world.
#[derive(Default)]
pub struct CommandBuffer {
    commands: Vec<Command>,
}

impl CommandBuffer {
    /// Create an empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue spawning a new entity; `init` is called with the new entity so
    /// the caller can insert components via another insert command.
    pub fn spawn(&mut self, init: impl FnOnce(&mut crate::ecs::world::World) + 'static) {
        self.commands.push(Command::Spawn(Box::new(init)));
    }

    /// Queue despawning `entity` (recursively removes all children).
    pub fn despawn(&mut self, entity: Entity) {
        self.commands.push(Command::Despawn(entity));
    }

    /// Queue inserting `component` on `entity`.
    pub fn insert<T: Component>(&mut self, entity: Entity, component: T) {
        self.commands.push(Command::InsertComponent {
            entity,
            applier: Box::new(move |world| {
                world.insert(entity, component);
            }),
        });
    }

    /// Queue removing component `T` from `entity`.
    pub fn remove<T: Component>(&mut self, entity: Entity) {
        self.commands.push(Command::RemoveComponent {
            entity,
            remover: Box::new(move |world| {
                world.remove::<T>(entity);
            }),
        });
    }

    /// Apply all queued commands to `world` and clear the buffer.
    pub fn flush(&mut self, world: &mut crate::ecs::world::World) {
        // Drain into a local vec so we can mutably borrow world inside the loop.
        let commands: Vec<_> = self.commands.drain(..).collect();
        commands.into_iter().for_each(|cmd| match cmd {
            Command::Spawn(init) => init(world),
            Command::Despawn(e) => { world.despawn(e); }
            Command::InsertComponent { applier, .. } => applier(world),
            Command::RemoveComponent { remover, .. } => remover(world),
        });
    }

    /// Returns `true` if there are no pending commands.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}
