//! Keeps carried flags attached to their carriers.

use crate::components::Transform;
use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::entity::Entity;
use crate::ecs::system::System;
use crate::ecs::world::World;
use crate::game::ctf::resources::{CarrierState, EntityRefs};
use crate::game::ctf::FLAG_CARRY_OFFSET_Y;
use crate::math::Vec3;

/// Moves carried flags to a visible offset under the carrier.
#[derive(Debug, Default)]
pub struct FlagCarrySystem;

impl System for FlagCarrySystem {
    fn run(&self, world: &World, commands: &mut CommandBuffer) {
        let Some(carrier) = world.resource::<CarrierState>().copied() else {
            return;
        };
        let Some(refs) = world.resource::<EntityRefs>().cloned() else {
            return;
        };

        if let Some(slot) = carrier.blue_flag_carrier {
            move_flag_to_carrier(world, commands, refs.player(slot), refs.blue_flag);
        }
        if let Some(slot) = carrier.red_flag_carrier {
            move_flag_to_carrier(world, commands, refs.player(slot), refs.red_flag);
        }
    }
}

fn move_flag_to_carrier(
    world: &World,
    commands: &mut CommandBuffer,
    carrier: Entity,
    flag: Entity,
) {
    let Some(carrier_tf) = world.get::<Transform>(carrier) else {
        return;
    };
    let Some(flag_tf) = world.get::<Transform>(flag) else {
        return;
    };

    let mut next_flag_tf = flag_tf.clone();
    next_flag_tf.position = Vec3::new(
        carrier_tf.position.x,
        carrier_tf.position.y + FLAG_CARRY_OFFSET_Y,
        flag_tf.position.z,
    );
    commands.insert(flag, next_flag_tf);
}
