//! Player-vs-wall collision for the CTF arena.

use crate::components::Transform;
use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::system::System;
use crate::ecs::world::World;
use crate::game::ctf::components::{PlayerMarker, Wall};
use crate::game::ctf::PLAYER_RADIUS;
use crate::math::Vec3;

/// Pushes player circles out of wall AABBs.
#[derive(Debug, Default)]
pub struct WallCollisionSystem;

impl System for WallCollisionSystem {
    fn run(&self, world: &World, commands: &mut CommandBuffer) {
        let walls: Vec<(f32, f32, f32, f32)> = world
            .query2::<Transform, Wall>()
            .map(|(_, transform, wall)| {
                (
                    transform.position.x,
                    transform.position.y,
                    wall.w,
                    wall.h,
                )
            })
            .collect();

        if walls.is_empty() {
            return;
        }

        for (entity, transform, _marker) in world.query2::<Transform, PlayerMarker>() {
            let mut x = transform.position.x;
            let mut y = transform.position.y;

            for (wall_x, wall_y, half_w, half_h) in &walls {
                resolve_circle_aabb(&mut x, &mut y, PLAYER_RADIUS, *wall_x, *wall_y, *half_w, *half_h);
            }

            if (x - transform.position.x).abs() > f32::EPSILON
                || (y - transform.position.y).abs() > f32::EPSILON
            {
                let mut corrected = transform.clone();
                corrected.position = Vec3::new(x, y, transform.position.z);
                commands.insert(entity, corrected);
            }
        }
    }
}

fn resolve_circle_aabb(
    circle_x: &mut f32,
    circle_y: &mut f32,
    radius: f32,
    wall_x: f32,
    wall_y: f32,
    half_w: f32,
    half_h: f32,
) {
    let min_x = wall_x - half_w;
    let max_x = wall_x + half_w;
    let min_y = wall_y - half_h;
    let max_y = wall_y + half_h;

    let closest_x = circle_x.clamp(min_x, max_x);
    let closest_y = circle_y.clamp(min_y, max_y);
    let dx = *circle_x - closest_x;
    let dy = *circle_y - closest_y;
    let dist_sq = dx * dx + dy * dy;
    let radius_sq = radius * radius;

    if dist_sq >= radius_sq {
        return;
    }

    if dist_sq > f32::EPSILON {
        let dist = dist_sq.sqrt();
        let push = radius - dist;
        *circle_x += dx / dist * push;
        *circle_y += dy / dist * push;
        return;
    }

    let left = (*circle_x - min_x).abs();
    let right = (max_x - *circle_x).abs();
    let top = (*circle_y - min_y).abs();
    let bottom = (max_y - *circle_y).abs();
    let nearest = left.min(right).min(top).min(bottom);

    if nearest == left {
        *circle_x = min_x - radius;
    } else if nearest == right {
        *circle_x = max_x + radius;
    } else if nearest == top {
        *circle_y = min_y - radius;
    } else {
        *circle_y = max_y + radius;
    }
}
