//! Message bus — routes loop-phase messages to registered systems.

use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::system::System;
use crate::ecs::world::World;
use super::message::LoopPhase;

struct Registration {
    phase: LoopPhase,
    priority: i32,
    system: Box<dyn System>,
}

/// Dispatches `LoopPhase` messages to registered systems.
///
/// Systems are called in ascending priority order within each phase.
/// The game loop calls [`MessageBus::run_frame`] once per frame; it sends
/// `First → Update → Last` in order, flushing the command buffer after each.
#[derive(Default)]
pub struct MessageBus {
    handlers: Vec<Registration>,
}

impl MessageBus {
    /// Create an empty bus.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a system to run during `phase` at the given `priority`.
    ///
    /// Lower priority numbers run first.  Registrations are unsorted until
    /// [`run_frame`](Self::run_frame) is called.
    pub fn register(&mut self, phase: LoopPhase, priority: i32, system: impl System + 'static) {
        self.handlers.push(Registration { phase, priority, system: Box::new(system) });
    }

    fn dispatch_phase(&self, phase: &LoopPhase, world: &World, commands: &mut CommandBuffer) {
        let mut indices: Vec<usize> = self.handlers
            .iter()
            .enumerate()
            .filter(|(_, h)| &h.phase == phase)
            .map(|(i, _)| i)
            .collect();
        indices.sort_by_key(|&i| self.handlers[i].priority);
        for i in indices {
            self.handlers[i].system.run(world, commands);
        }
    }

    /// Run one full frame: `First`, then `Update`, then `Last`.
    ///
    /// The [`CommandBuffer`] is flushed after each phase so that mutations
    /// applied in `First` are visible to `Update` systems, etc.
    pub fn run_frame(&self, world: &mut World) {
        for phase in [LoopPhase::First, LoopPhase::Update, LoopPhase::Last] {
            let mut cmds = CommandBuffer::new();
            self.dispatch_phase(&phase, world, &mut cmds);
            cmds.flush(world);
        }
    }
}
