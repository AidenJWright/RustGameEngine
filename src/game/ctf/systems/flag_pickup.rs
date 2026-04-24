//! Flag pickup detection.

use crate::components::Transform;
use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::system::System;
use crate::ecs::world::World;
use crate::game::ctf::resources::{CarrierState, EntityRefs};
use crate::game::ctf::{FLAG_RADIUS, PLAYER_RADIUS};

use super::{distance_sq, is_playing};

/// Lets each player pick up the opposing flag by overlapping it.
#[derive(Debug, Default)]
pub struct FlagPickupSystem;

impl System for FlagPickupSystem {
    fn run(&self, world: &World, commands: &mut CommandBuffer) {
        if !is_playing(world) {
            return;
        }

        let Some(refs) = world.resource::<EntityRefs>().copied() else {
            return;
        };
        let mut carrier = world.resource::<CarrierState>().copied().unwrap_or_default();
        let original = carrier;

        let Some(p1_tf) = world.get::<Transform>(refs.p1) else {
            return;
        };
        let Some(p2_tf) = world.get::<Transform>(refs.p2) else {
            return;
        };
        let Some(p1_flag_tf) = world.get::<Transform>(refs.p1_flag) else {
            return;
        };
        let Some(p2_flag_tf) = world.get::<Transform>(refs.p2_flag) else {
            return;
        };

        let pickup_range_sq = (PLAYER_RADIUS + FLAG_RADIUS) * (PLAYER_RADIUS + FLAG_RADIUS);

        if !carrier.p1_carries
            && distance_sq(
                p1_tf.position.x,
                p1_tf.position.y,
                p2_flag_tf.position.x,
                p2_flag_tf.position.y,
            ) < pickup_range_sq
        {
            carrier.p1_carries = true;
        }

        if !carrier.p2_carries
            && distance_sq(
                p2_tf.position.x,
                p2_tf.position.y,
                p1_flag_tf.position.x,
                p1_flag_tf.position.y,
            ) < pickup_range_sq
        {
            carrier.p2_carries = true;
        }

        if carrier.p1_carries != original.p1_carries
            || carrier.p2_carries != original.p2_carries
        {
            commands.insert_resource(carrier);
        }
    }
}
