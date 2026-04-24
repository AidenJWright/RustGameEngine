//! `PlayerInputSystem` — reads `PlayerInput` key bindings and `KeysPressed`
//! resource to produce `Velocity` updates for player-controlled entities.
//!
//! Dead-reckoning flow:
//! 1. Each frame the client applies its own input immediately (local prediction).
//! 2. Input is also sent to the host via the multiplayer session.
//! 3. Host is the source of truth; corrections reconcile the client state.
//! 4. Smooth correction interpolation is handled in the game loop.

use crate::components::{PlayerInput, Velocity};
use crate::components::player_input::ConfigKey;
use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::resource::KeysPressed;
use crate::ecs::system::System;
use crate::ecs::world::World;
use crate::messaging::LoopPhase;

/// Maps `ConfigKey` to its raw `u32` discriminant used in `KeysPressed`.
///
/// The discriminant must match what `demo/game.rs` stores when it processes
/// `WindowEvent::KeyboardInput`.
pub fn config_key_discriminant(key: ConfigKey) -> u32 {
    match key {
        ConfigKey::None => u32::MAX,
        ConfigKey::ArrowLeft => 0,
        ConfigKey::ArrowRight => 1,
        ConfigKey::ArrowUp => 2,
        ConfigKey::ArrowDown => 3,
        ConfigKey::W => 4,
        ConfigKey::A => 5,
        ConfigKey::S => 6,
        ConfigKey::D => 7,
        ConfigKey::R => 11,
        ConfigKey::Space => 8,
        ConfigKey::Return => 9,
    }
}

/// Drives velocity on entities that carry a `PlayerInput` component.
///
/// Reads `KeysPressed` (updated by the game loop) and writes `Velocity` via
/// the command buffer.  `MovementSystem` then integrates velocity into position.
#[derive(Debug, Default)]
pub struct PlayerInputSystem;

impl PlayerInputSystem {
    /// Recommended registration phase.
    pub const PHASE: LoopPhase = LoopPhase::Update;
    /// Recommended registration priority — run before MovementSystem (priority 0).
    pub const PRIORITY: i32 = -10;
}

impl System for PlayerInputSystem {
    fn run(&self, world: &World, commands: &mut CommandBuffer) {
        let keys = match world.resource::<KeysPressed>() {
            Some(k) => k.clone(),
            None => return,
        };

        world
            .query2::<PlayerInput, Velocity>()
            .map(|(entity, pi, _vel)| {
                // Compute axis values.
                let neg_h = keys.is_held(config_key_discriminant(pi.horizontal.negative));
                let pos_h = keys.is_held(config_key_discriminant(pi.horizontal.positive));
                let neg_v = keys.is_held(config_key_discriminant(pi.vertical.negative));
                let pos_v = keys.is_held(config_key_discriminant(pi.vertical.positive));

                let raw_x = (if pos_h { 1.0_f32 } else { 0.0 })
                    - (if neg_h { 1.0_f32 } else { 0.0 });
                let raw_y = (if pos_v { 1.0_f32 } else { 0.0 })
                    - (if neg_v { 1.0_f32 } else { 0.0 });

                // Normalise diagonal.
                let len = (raw_x * raw_x + raw_y * raw_y).sqrt();
                let (nx, ny) = if len > 1.0 {
                    (raw_x / len, raw_y / len)
                } else {
                    (raw_x, raw_y)
                };

                let new_vel = Velocity {
                    dx: nx * pi.speed,
                    dy: ny * pi.speed,
                };
                (entity, new_vel)
            })
            .for_each(|(entity, v)| {
                commands.insert(entity, v);
            });
    }
}
