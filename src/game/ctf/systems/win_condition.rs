//! Win detection and post-win movement halt.

use crate::components::{Transform, Velocity};
use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::system::System;
use crate::ecs::world::World;
use crate::game::ctf::components::PlayerMarker;
use crate::game::ctf::resources::{CarrierState, EntityRefs, GamePhase, GameState};
use crate::game::ctf::MIDLINE_X;

use super::is_playing;

/// Declares a winner once a carrier returns to their own side.
#[derive(Debug, Default)]
pub struct WinConditionSystem;

impl System for WinConditionSystem {
    fn run(&self, world: &World, commands: &mut CommandBuffer) {
        if !is_playing(world) {
            return;
        }

        let Some(refs) = world.resource::<EntityRefs>().copied() else {
            return;
        };
        let carrier = world.resource::<CarrierState>().copied().unwrap_or_default();

        let Some(p1_tf) = world.get::<Transform>(refs.p1) else {
            return;
        };
        let Some(p2_tf) = world.get::<Transform>(refs.p2) else {
            return;
        };

        let winner = if carrier.p1_carries && p1_tf.position.x < MIDLINE_X {
            Some(1)
        } else if carrier.p2_carries && p2_tf.position.x > MIDLINE_X {
            Some(2)
        } else {
            None
        };

        if let Some(id) = winner {
            commands.insert_resource(GameState {
                phase: GamePhase::Won(id),
            });
        }
    }
}

/// Zeroes player velocity after the game has ended.
#[derive(Debug, Default)]
pub struct StopOnWinSystem;

impl System for StopOnWinSystem {
    fn run(&self, world: &World, commands: &mut CommandBuffer) {
        if is_playing(world) {
            return;
        }

        for (entity, _marker, velocity) in world.query2::<PlayerMarker, Velocity>() {
            if velocity.dx != 0.0 || velocity.dy != 0.0 {
                commands.insert(entity, Velocity { dx: 0.0, dy: 0.0 });
            }
        }
    }
}
