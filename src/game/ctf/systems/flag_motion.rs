//! Thrown flag movement and obstacle stopping.

use crate::components::Transform;
use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::resource::DeltaTime;
use crate::ecs::system::System;
use crate::ecs::world::World;
use crate::game::ctf::components::Wall;
use crate::game::ctf::resources::{CtfSyncState, EntityRefs, FlagMotion, FlagMotionState};
use crate::game::ctf::{FLAG_RADIUS, FLAG_THROW_SPEED};
use crate::math::Vec3;

use super::is_playing;

/// Advances flags that have been thrown by a carrier.
#[derive(Debug, Default)]
pub struct FlagMotionSystem;

impl System for FlagMotionSystem {
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
        let mut motion = world
            .resource::<FlagMotionState>()
            .copied()
            .unwrap_or_default();
        let original = motion;
        let walls = world
            .query2::<Transform, Wall>()
            .map(|(_, transform, wall)| {
                (transform.position.x, transform.position.y, wall.w, wall.h)
            })
            .collect::<Vec<_>>();

        if let Some(red_motion) = motion.red.as_mut() {
            advance_flag(world, commands, refs.red_flag, red_motion, &walls, dt);
            if red_motion.remaining_distance <= f32::EPSILON {
                motion.red = None;
            }
        }

        if let Some(blue_motion) = motion.blue.as_mut() {
            advance_flag(world, commands, refs.blue_flag, blue_motion, &walls, dt);
            if blue_motion.remaining_distance <= f32::EPSILON {
                motion.blue = None;
            }
        }

        if motion != original {
            commands.insert_resource(motion);
        }
        if original.red.is_some() && motion.red.is_none()
            || original.blue.is_some() && motion.blue.is_none()
        {
            commands.insert_resource(CtfSyncState::dirty());
        }
    }
}

fn advance_flag(
    world: &World,
    commands: &mut CommandBuffer,
    flag: crate::ecs::entity::Entity,
    motion: &mut FlagMotion,
    walls: &[(f32, f32, f32, f32)],
    dt: f32,
) {
    let Some(transform) = world.get::<Transform>(flag) else {
        motion.remaining_distance = 0.0;
        return;
    };

    let step = (FLAG_THROW_SPEED * dt).min(motion.remaining_distance);
    let next_x = transform.position.x + motion.dir_x * step;
    let next_y = transform.position.y + motion.dir_y * step;

    if flag_hits_wall(next_x, next_y, walls) {
        motion.remaining_distance = 0.0;
        return;
    }

    let mut next = transform.clone();
    next.position = Vec3::new(next_x, next_y, transform.position.z);
    commands.insert(flag, next);
    motion.remaining_distance -= step;
}

fn flag_hits_wall(x: f32, y: f32, walls: &[(f32, f32, f32, f32)]) -> bool {
    walls.iter().any(|(wall_x, wall_y, half_w, half_h)| {
        x >= wall_x - half_w - FLAG_RADIUS
            && x <= wall_x + half_w + FLAG_RADIUS
            && y >= wall_y - half_h - FLAG_RADIUS
            && y <= wall_y + half_h + FLAG_RADIUS
    })
}
