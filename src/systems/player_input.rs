//! `PlayerInputSystem` — reads `PlayerInput` key bindings and `KeysPressed`
//! resource to produce `Velocity` updates for player-controlled entities.
//!
//! Dead-reckoning flow:
//! 1. Each frame the client applies its own input immediately (local prediction).
//! 2. Input is also sent to the host via the multiplayer session.
//! 3. Host is the source of truth; corrections reconcile the client state.
//! 4. Smooth correction interpolation is handled in the game loop.

use crate::components::player_input::ConfigKey;
use crate::components::{PlayerInput, Velocity};
use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::resource::KeysPressed;
use crate::ecs::system::System;
use crate::ecs::world::World;
use crate::messaging::LoopPhase;
use crate::platform::KeyCode;

/// Maps `ConfigKey` to its raw `u32` discriminant used in `KeysPressed`.
///
/// The discriminant must match what `key_code_discriminant` stores in
/// `KeysPressed` when the game loop processes keyboard input.
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
        ConfigKey::Digit1 => 12,
        ConfigKey::Digit2 => 13,
        ConfigKey::Digit3 => 14,
        ConfigKey::Digit4 => 15,
        ConfigKey::LeftShift => 10,
        ConfigKey::Space => 8,
        ConfigKey::Return => 9,
    }
}

/// Convert an engine key code into a saved `PlayerInput` binding.
pub fn config_key_from_key_code(code: KeyCode) -> Option<ConfigKey> {
    match code {
        KeyCode::Left => Some(ConfigKey::ArrowLeft),
        KeyCode::Right => Some(ConfigKey::ArrowRight),
        KeyCode::Up => Some(ConfigKey::ArrowUp),
        KeyCode::Down => Some(ConfigKey::ArrowDown),
        KeyCode::W => Some(ConfigKey::W),
        KeyCode::A => Some(ConfigKey::A),
        KeyCode::S => Some(ConfigKey::S),
        KeyCode::D => Some(ConfigKey::D),
        KeyCode::R => Some(ConfigKey::R),
        KeyCode::Digit1 => Some(ConfigKey::Digit1),
        KeyCode::Digit2 => Some(ConfigKey::Digit2),
        KeyCode::Digit3 => Some(ConfigKey::Digit3),
        KeyCode::Digit4 => Some(ConfigKey::Digit4),
        KeyCode::LeftShift => Some(ConfigKey::LeftShift),
        KeyCode::Space => Some(ConfigKey::Space),
        KeyCode::Return => Some(ConfigKey::Return),
        _ => None,
    }
}

/// Convert an engine key code into the discriminant stored by `KeysPressed`.
pub fn key_code_discriminant(code: KeyCode) -> Option<u32> {
    config_key_from_key_code(code).map(config_key_discriminant)
}

/// Drives velocity on entities that carry a `PlayerInput` component.
///
/// Reads `KeysPressed` (updated by the game loop) and writes `Velocity` via
/// the command buffer. Entities do not need to be saved with `Velocity`;
/// `MovementSystem` then integrates the generated velocity into position.
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
            .query::<PlayerInput>()
            .map(|(entity, pi)| {
                // Compute axis values.
                let neg_h = keys.is_held(config_key_discriminant(pi.horizontal.negative));
                let pos_h = keys.is_held(config_key_discriminant(pi.horizontal.positive));
                let neg_v = keys.is_held(config_key_discriminant(pi.vertical.negative));
                let pos_v = keys.is_held(config_key_discriminant(pi.vertical.positive));

                let raw_x =
                    (if pos_h { 1.0_f32 } else { 0.0 }) - (if neg_h { 1.0_f32 } else { 0.0 });
                let raw_y =
                    (if pos_v { 1.0_f32 } else { 0.0 }) - (if neg_v { 1.0_f32 } else { 0.0 });

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{ConfigKey, Transform, Velocity};
    use crate::ecs::entity::Entity;
    use crate::ecs::resource::DeltaTime;
    use crate::messaging::{LoopPhase, MessageBus};
    use crate::platform::KeyCode;
    use crate::scene::{reload_scene, DEFAULT_SCENE_PATH};
    use crate::systems::movement::MovementSystem;

    #[test]
    fn config_key_capture_mapping_matches_runtime_discriminants() {
        assert_eq!(
            config_key_from_key_code(KeyCode::Left),
            Some(ConfigKey::ArrowLeft)
        );
        assert_eq!(
            config_key_from_key_code(KeyCode::Digit4),
            Some(ConfigKey::Digit4)
        );
        assert_eq!(
            key_code_discriminant(KeyCode::Digit4),
            Some(config_key_discriminant(ConfigKey::Digit4))
        );
        assert_eq!(config_key_from_key_code(KeyCode::Escape), None);
    }

    #[test]
    fn default_scene_player_input_entities_move_without_saved_velocity() {
        let mut world = World::new();
        reload_scene(&mut world, DEFAULT_SCENE_PATH).expect("load default scene");

        let controlled: Vec<Entity> = world
            .query::<PlayerInput>()
            .map(|(entity, _)| entity)
            .collect();
        assert_eq!(controlled.len(), 2);
        assert!(controlled
            .iter()
            .all(|&entity| world.get::<Velocity>(entity).is_none()));

        let starting_positions: Vec<(Entity, _)> = controlled
            .iter()
            .map(|&entity| {
                (
                    entity,
                    world
                        .get::<Transform>(entity)
                        .expect("controlled entity has transform")
                        .position,
                )
            })
            .collect();

        let mut keys = KeysPressed::default();
        keys.press(config_key_discriminant(ConfigKey::A));
        keys.press(config_key_discriminant(ConfigKey::ArrowRight));
        world.insert_resource(keys);
        world.insert_resource(DeltaTime(1.0));

        let mut bus = MessageBus::new();
        bus.register(
            LoopPhase::Update,
            PlayerInputSystem::PRIORITY,
            PlayerInputSystem,
        );
        bus.register(LoopPhase::Update, MovementSystem::PRIORITY, MovementSystem);
        bus.run_frame(&mut world);

        for (entity, start) in starting_positions {
            assert!(world.get::<Velocity>(entity).is_some());

            let end = world
                .get::<Transform>(entity)
                .expect("controlled entity still has transform")
                .position;
            assert!(
                end.x > start.x,
                "entity {entity} should move right after its saved binding is pressed"
            );
        }
    }
}
