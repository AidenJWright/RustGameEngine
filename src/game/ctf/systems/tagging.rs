//! Defensive tagging logic.

use crate::components::Transform;
use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::resource::KeysPressed;
use crate::ecs::system::System;
use crate::ecs::world::World;
use crate::game::ctf::resources::{CarrierState, EntityRefs, TagInputState};
use crate::game::ctf::{MIDLINE_X, P1_TAG_KEY, P2_TAG_KEY, TAG_RANGE};
use crate::math::Vec3;

use super::{distance_sq, is_playing};

/// Tags a flag carrier on the defender's side when the tag key is pressed.
#[derive(Debug, Default)]
pub struct TaggingSystem;

impl System for TaggingSystem {
    fn run(&self, world: &World, commands: &mut CommandBuffer) {
        let keys = world.resource::<KeysPressed>().cloned().unwrap_or_default();
        let previous = world.resource::<TagInputState>().copied().unwrap_or_default();
        let p1_down = keys.is_held(P1_TAG_KEY);
        let p2_down = keys.is_held(P2_TAG_KEY);
        let p1_pressed = p1_down && !previous.p1_tag_down;
        let p2_pressed = p2_down && !previous.p2_tag_down;

        commands.insert_resource(TagInputState {
            p1_tag_down: p1_down,
            p2_tag_down: p2_down,
        });

        if !is_playing(world) {
            return;
        }

        let Some(refs) = world.resource::<EntityRefs>().copied() else {
            return;
        };
        let mut carrier = world.resource::<CarrierState>().copied().unwrap_or_default();
        let original = carrier;

        let Some(p1_tf) = world.get::<Transform>(refs.p1).cloned() else {
            return;
        };
        let Some(p2_tf) = world.get::<Transform>(refs.p2).cloned() else {
            return;
        };

        let tag_range_sq = TAG_RANGE * TAG_RANGE;
        let players_in_range = distance_sq(
            p1_tf.position.x,
            p1_tf.position.y,
            p2_tf.position.x,
            p2_tf.position.y,
        ) < tag_range_sq;

        if p1_pressed && carrier.p2_carries && p2_tf.position.x < MIDLINE_X && players_in_range {
            carrier.p2_carries = false;
            drop_flag_at(world, commands, refs.p1_flag, &p2_tf);
            teleport_player(world, commands, refs.p2, refs.p2_spawn);
        }

        if p2_pressed && carrier.p1_carries && p1_tf.position.x > MIDLINE_X && players_in_range {
            carrier.p1_carries = false;
            drop_flag_at(world, commands, refs.p2_flag, &p1_tf);
            teleport_player(world, commands, refs.p1, refs.p1_spawn);
        }

        if carrier.p1_carries != original.p1_carries
            || carrier.p2_carries != original.p2_carries
        {
            commands.insert_resource(carrier);
        }
    }
}

fn drop_flag_at(
    world: &World,
    commands: &mut CommandBuffer,
    flag: crate::ecs::entity::Entity,
    carrier_tf: &Transform,
) {
    if let Some(flag_tf) = world.get::<Transform>(flag) {
        let mut dropped = flag_tf.clone();
        dropped.position = Vec3::new(
            carrier_tf.position.x,
            carrier_tf.position.y,
            flag_tf.position.z,
        );
        commands.insert(flag, dropped);
    }
}

fn teleport_player(
    world: &World,
    commands: &mut CommandBuffer,
    player: crate::ecs::entity::Entity,
    spawn: (f32, f32),
) {
    if let Some(player_tf) = world.get::<Transform>(player) {
        let mut teleported = player_tf.clone();
        teleported.position = Vec3::new(spawn.0, spawn.1, player_tf.position.z);
        commands.insert(player, teleported);
    }
}
