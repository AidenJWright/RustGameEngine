//! Flag pickup detection.

use crate::components::Transform;
use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::system::System;
use crate::ecs::world::World;
use crate::game::ctf::resources::{
    CarrierState, CtfSyncState, EntityRefs, FlagMotion, FlagMotionState,
};
use crate::game::ctf::FLAG_KNOCKDOWN_RANGE;
use crate::multiplayer::matchmaking::CtfSlot;

use super::{distance_sq, is_playing};

/// Lets each player pick up an available flag by overlapping it.
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
        let mut carrier = world
            .resource::<CarrierState>()
            .copied()
            .unwrap_or_default();
        let original = carrier;
        let flag_motion = world
            .resource::<FlagMotionState>()
            .copied()
            .unwrap_or_default();

        let Some(red_flag_tf) = world.get::<Transform>(refs.red_flag) else {
            return;
        };
        let Some(blue_flag_tf) = world.get::<Transform>(refs.blue_flag) else {
            return;
        };

        let pickup_range_sq = FLAG_KNOCKDOWN_RANGE * FLAG_KNOCKDOWN_RANGE;

        if flag_is_available(carrier.blue_flag_carrier, flag_motion.blue) {
            carrier.blue_flag_carrier = first_overlapping_slot(
                world,
                &refs,
                &CtfSlot::RED,
                blue_flag_tf,
                pickup_range_sq,
                &carrier,
            );
        }

        if flag_is_available(carrier.red_flag_carrier, flag_motion.red) {
            carrier.red_flag_carrier = first_overlapping_slot(
                world,
                &refs,
                &CtfSlot::BLUE,
                red_flag_tf,
                pickup_range_sq,
                &carrier,
            );
        }

        if flag_is_available(carrier.red_flag_carrier, flag_motion.red) {
            carrier.red_flag_carrier = first_overlapping_slot(
                world,
                &refs,
                &CtfSlot::RED,
                red_flag_tf,
                pickup_range_sq,
                &carrier,
            );
        }

        if flag_is_available(carrier.blue_flag_carrier, flag_motion.blue) {
            carrier.blue_flag_carrier = first_overlapping_slot(
                world,
                &refs,
                &CtfSlot::BLUE,
                blue_flag_tf,
                pickup_range_sq,
                &carrier,
            );
        }

        if carrier.blue_flag_carrier != original.blue_flag_carrier
            || carrier.red_flag_carrier != original.red_flag_carrier
        {
            commands.insert_resource(carrier);
            commands.insert_resource(CtfSyncState::dirty());
        }
    }
}

fn flag_is_available(carrier: Option<CtfSlot>, motion: Option<FlagMotion>) -> bool {
    carrier.is_none() && motion.is_none()
}

fn first_overlapping_slot(
    world: &World,
    refs: &EntityRefs,
    slots: &[CtfSlot],
    flag_tf: &Transform,
    pickup_range_sq: f32,
    carrier: &CarrierState,
) -> Option<CtfSlot> {
    slots.iter().copied().find(|slot| {
        if carries_any_flag(*slot, carrier) {
            return false;
        }
        let Some(player_tf) = world.get::<Transform>(refs.player(*slot)) else {
            return false;
        };
        distance_sq(
            player_tf.position.x,
            player_tf.position.y,
            flag_tf.position.x,
            flag_tf.position.y,
        ) <= pickup_range_sq
    })
}

fn carries_any_flag(slot: CtfSlot, carrier: &CarrierState) -> bool {
    carrier.red_flag_carrier == Some(slot) || carrier.blue_flag_carrier == Some(slot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::command_buffer::CommandBuffer;
    use crate::game::ctf::setup::setup_ctf_scene_entities;
    use crate::multiplayer::matchmaking::{CtfSlotAssignment, MapSize};
    use crate::scene::reload_scene;

    fn test_world() -> World {
        let mut world = World::new();
        reload_scene(&mut world, "assets/ctf_arena_small.json").expect("load CTF scene");
        setup_ctf_scene_entities(
            &mut world,
            &[
                CtfSlotAssignment {
                    client_id: 1,
                    primary_slot: CtfSlot::Red1,
                },
                CtfSlotAssignment {
                    client_id: 2,
                    primary_slot: CtfSlot::Blue1,
                },
            ],
            MapSize::Small,
        );
        world
    }

    #[test]
    fn player_can_pick_up_own_flag_when_empty_handed() {
        let mut world = test_world();
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
        let red = refs.player(CtfSlot::Red1);
        let red_flag = world
            .get::<Transform>(refs.red_flag)
            .cloned()
            .expect("red flag");
        let mut red_tf = world.get::<Transform>(red).cloned().expect("red player");
        red_tf.position.x = red_flag.position.x;
        red_tf.position.y = red_flag.position.y;
        world.insert(red, red_tf);

        let mut commands = CommandBuffer::new();
        FlagPickupSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        let carrier = world.resource::<CarrierState>().expect("carrier");
        assert_eq!(carrier.red_flag_carrier, Some(CtfSlot::Red1));
    }

    #[test]
    fn pickup_radius_matches_knockdown_radius() {
        let mut world = test_world();
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
        let red = refs.player(CtfSlot::Red1);
        let blue_flag = world
            .get::<Transform>(refs.blue_flag)
            .cloned()
            .expect("blue flag");
        let mut red_tf = world.get::<Transform>(red).cloned().expect("red player");
        red_tf.position.x = blue_flag.position.x - FLAG_KNOCKDOWN_RANGE;
        red_tf.position.y = blue_flag.position.y;
        world.insert(red, red_tf);

        let mut commands = CommandBuffer::new();
        FlagPickupSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        let carrier = world.resource::<CarrierState>().expect("carrier");
        assert_eq!(carrier.blue_flag_carrier, Some(CtfSlot::Red1));
    }

    #[test]
    fn player_cannot_pick_up_own_flag_while_carrying_another_flag() {
        let mut world = test_world();
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
        let red = refs.player(CtfSlot::Red1);
        let red_flag = world
            .get::<Transform>(refs.red_flag)
            .cloned()
            .expect("red flag");
        let mut red_tf = world.get::<Transform>(red).cloned().expect("red player");
        red_tf.position.x = red_flag.position.x;
        red_tf.position.y = red_flag.position.y;
        world.insert(red, red_tf);
        world.insert_resource(CarrierState {
            red_flag_carrier: None,
            blue_flag_carrier: Some(CtfSlot::Red1),
        });

        let mut commands = CommandBuffer::new();
        FlagPickupSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        let carrier = world.resource::<CarrierState>().expect("carrier");
        assert_eq!(carrier.red_flag_carrier, None);
        assert_eq!(carrier.blue_flag_carrier, Some(CtfSlot::Red1));
    }
}
