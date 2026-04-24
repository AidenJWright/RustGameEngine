//! MOBA-style point-and-click movement for CTF player slots.

use crate::components::{Transform, Velocity};
use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::resource::DeltaTime;
use crate::ecs::system::System;
use crate::ecs::world::World;
use crate::game::ctf::resources::{AutoMoveState, CtfSyncState, EntityRefs};
use crate::game::ctf::{AUTO_MOVE_SPEED, PATH_WAYPOINT_RADIUS};
use crate::multiplayer::matchmaking::CtfSlot;

use super::is_playing;

/// Drives player velocity along any active auto-move paths.
#[derive(Debug, Default)]
pub struct AutoMoveSystem;

impl System for AutoMoveSystem {
    fn run(&self, world: &World, commands: &mut CommandBuffer) {
        if !is_playing(world) {
            return;
        }

        let dt = world.resource::<DeltaTime>().copied().unwrap_or_default().0;
        if dt <= f32::EPSILON {
            return;
        }

        let Some(refs) = world.resource::<EntityRefs>().cloned() else {
            return;
        };
        let mut auto_move = world.resource::<AutoMoveState>().cloned().unwrap_or_default();
        let mut changed = false;
        let mut finished = false;

        for slot in CtfSlot::ALL {
            let Some(path) = auto_move.paths[slot.index()].as_mut() else {
                continue;
            };
            let Some(transform) = world.get::<Transform>(refs.player(slot)) else {
                continue;
            };

            while path.next_index < path.waypoints.len() {
                let target = path.waypoints[path.next_index];
                let dx = target.0 - transform.position.x;
                let dy = target.1 - transform.position.y;
                if dx * dx + dy * dy > PATH_WAYPOINT_RADIUS * PATH_WAYPOINT_RADIUS {
                    break;
                }
                path.next_index += 1;
                changed = true;
            }

            if path.next_index >= path.waypoints.len() {
                auto_move.paths[slot.index()] = None;
                commands.insert(refs.player(slot), Velocity { dx: 0.0, dy: 0.0 });
                changed = true;
                finished = true;
                continue;
            }

            let target = path.waypoints[path.next_index];
            let dx = target.0 - transform.position.x;
            let dy = target.1 - transform.position.y;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist <= f32::EPSILON {
                continue;
            }
            let speed = AUTO_MOVE_SPEED.min(dist / dt);
            commands.insert(
                refs.player(slot),
                Velocity {
                    dx: dx / dist * speed,
                    dy: dy / dist * speed,
                },
            );
        }

        if changed {
            commands.insert_resource(auto_move);
        }
        if finished {
            commands.insert_resource(CtfSyncState { dirty: true });
        }
    }
}
