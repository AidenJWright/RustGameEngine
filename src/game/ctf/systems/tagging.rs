//! Tagging is applied by `CtfInputSystem`.
//!
//! This compatibility system intentionally does no work; networked and local
//! CTF both feed tag button state into `CtfInputState`.

use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::system::System;
use crate::ecs::world::World;

/// Compatibility shim for older registration sites.
#[derive(Debug, Default)]
pub struct TaggingSystem;

impl System for TaggingSystem {
    fn run(&self, _world: &World, _commands: &mut CommandBuffer) {}
}
