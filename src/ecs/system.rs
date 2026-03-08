//! System trait, scheduler, and function-pointer system adapter.

use super::command_buffer::CommandBuffer;
use super::world::World;

// ---------------------------------------------------------------------------
// System trait
// ---------------------------------------------------------------------------

/// A system reads world state and optionally queues commands.
///
/// Systems are zero-size structs or closures — they own **no** mutable state.
/// All state lives in components or resources on the `World`.
pub trait System: Send + Sync {
    /// Execute one frame of logic.
    ///
    /// * `world` — read-only access (queries, resource reads)
    /// * `commands` — deferred mutations flushed after this call returns
    fn run(&self, world: &World, commands: &mut CommandBuffer);
}

// ---------------------------------------------------------------------------
// Function-pointer adapter
// ---------------------------------------------------------------------------

/// Wraps a plain function as a [`System`].
///
/// This lets you register `fn run_movement(world: &World, cmds: &mut CommandBuffer)`
/// directly without defining a struct.
pub struct FnSystem(pub fn(&World, &mut CommandBuffer));

impl System for FnSystem {
    fn run(&self, world: &World, commands: &mut CommandBuffer) {
        (self.0)(world, commands);
    }
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// Runs registered systems in insertion order, flushing the command buffer
/// after each one so that despawned/spawned entities are visible to later systems.
#[derive(Default)]
pub struct Scheduler {
    systems: Vec<Box<dyn System>>,
}

impl Scheduler {
    /// Create an empty scheduler.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a system to run every tick.
    pub fn add_system(&mut self, system: impl System + 'static) {
        self.systems.push(Box::new(system));
    }

    /// Register a plain function as a system.
    pub fn add_fn(&mut self, f: fn(&World, &mut CommandBuffer)) {
        self.systems.push(Box::new(FnSystem(f)));
    }

    /// Run all systems in order, flushing commands between each.
    pub fn run_all(&self, world: &mut World) {
        let mut commands = CommandBuffer::new();
        self.systems.iter().for_each(|system| {
            system.run(world, &mut commands);
            commands.flush(world);
        });
    }
}
