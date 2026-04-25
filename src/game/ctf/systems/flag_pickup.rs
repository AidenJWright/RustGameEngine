//! Flag pickup detection.

use crate::components::Transform;
use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::system::System;
use crate::ecs::world::World;
use crate::game::ctf::resources::{CarrierState, CtfSyncState, EntityRefs, FlagMotionState};
use crate::game::ctf::{FLAG_RADIUS, PLAYER_RADIUS};
use crate::multiplayer::matchmaking::CtfSlot;

use super::{distance_sq, is_playing};

/// Lets each player pick up the opposing flag by overlapping it.
#[derive(Debug, Default)]
pub struct FlagPickupSystem;

impl System for FlagPickupSystem {
    fn run(&self, world: &World, commands: &mut CommandBuffer) {
        if !is_playing(world) {
            return;
        }

        let Some(refs) = world.resource::<EntityRefs>().cloned() else {
            return;
        };
        let mut carrier = world.resource::<CarrierState>().copied().unwrap_or_default();
        let original = carrier;
        let flag_motion = world.resource::<FlagMotionState>().copied().unwrap_or_default();

        let Some(red_flag_tf) = world.get::<Transform>(refs.red_flag) else {
            return;
        };
        let Some(blue_flag_tf) = world.get::<Transform>(refs.blue_flag) else {
            return;
        };

        let pickup_range_sq = (PLAYER_RADIUS + FLAG_RADIUS) * (PLAYER_RADIUS + FLAG_RADIUS);

        if carrier.blue_flag_carrier.is_none() && flag_motion.blue.is_none() {
            carrier.blue_flag_carrier =
                first_overlapping_slot(world, &refs, &CtfSlot::RED, blue_flag_tf, pickup_range_sq);
        }

        if carrier.red_flag_carrier.is_none() && flag_motion.red.is_none() {
            carrier.red_flag_carrier =
                first_overlapping_slot(world, &refs, &CtfSlot::BLUE, red_flag_tf, pickup_range_sq);
        }

        if carrier.blue_flag_carrier != original.blue_flag_carrier
            || carrier.red_flag_carrier != original.red_flag_carrier
        {
            commands.insert_resource(carrier);
            commands.insert_resource(CtfSyncState { dirty: true });
        }
    }
}

fn first_overlapping_slot(
    world: &World,
    refs: &EntityRefs,
    slots: &[CtfSlot],
    flag_tf: &Transform,
    pickup_range_sq: f32,
) -> Option<CtfSlot> {
    slots.iter().copied().find(|slot| {
        let Some(player_tf) = world.get::<Transform>(refs.player(*slot)) else {
            return false;
        };
        distance_sq(
            player_tf.position.x,
            player_tf.position.y,
            flag_tf.position.x,
            flag_tf.position.y,
        ) < pickup_range_sq
    })
}
