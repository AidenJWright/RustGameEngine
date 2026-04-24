//! Win detection and post-win movement halt.

use crate::components::{Transform, Velocity};
use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::system::System;
use crate::ecs::world::World;
use crate::game::ctf::components::PlayerMarker;
use crate::game::ctf::resources::{CarrierState, CtfSyncState, EntityRefs, GamePhase, GameState};
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

        let Some(refs) = world.resource::<EntityRefs>().cloned() else {
            return;
        };
        let carrier = world.resource::<CarrierState>().copied().unwrap_or_default();

        let winner = carrier
            .blue_flag_carrier
            .and_then(|slot| {
                world
                    .get::<Transform>(refs.player(slot))
                    .filter(|tf| tf.position.x < MIDLINE_X)
                    .map(|_| 1)
            })
            .or_else(|| {
                carrier.red_flag_carrier.and_then(|slot| {
                    world
                        .get::<Transform>(refs.player(slot))
                        .filter(|tf| tf.position.x > MIDLINE_X)
                        .map(|_| 2)
                })
            });

        if let Some(id) = winner {
            commands.insert_resource(GameState {
                phase: GamePhase::Won(id),
            });
            commands.insert_resource(CtfSyncState { dirty: true });
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
