//! Automatic CTF tagging and thrown-flag knockdown.

use crate::components::{Transform, Velocity};
use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::system::System;
use crate::ecs::world::World;
use crate::game::ctf::resources::{CarrierState, CtfSyncState, EntityRefs, FlagMotionState};
use crate::game::ctf::{FLAG_KNOCKDOWN_RANGE, TAG_RANGE};
use crate::math::Vec3;
use crate::multiplayer::matchmaking::{CtfSlot, MapSize};

use super::{distance_sq, is_playing};

/// Tags opponents and stops moving flags as soon as they enter range.
#[derive(Debug, Default)]
pub struct TaggingSystem;

impl System for TaggingSystem {
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
        let mut flag_motion = world
            .resource::<FlagMotionState>()
            .copied()
            .unwrap_or_default();
        let mut player_transforms = std::array::from_fn(|idx| {
            world
                .get::<Transform>(refs.players[idx])
                .cloned()
                .unwrap_or_else(Transform::identity)
        });
        let mut velocities = std::array::from_fn(|idx| {
            world
                .get::<Velocity>(refs.players[idx])
                .cloned()
                .unwrap_or(Velocity { dx: 0.0, dy: 0.0 })
        });
        let mut red_flag_tf = world
            .get::<Transform>(refs.red_flag)
            .cloned()
            .unwrap_or_else(Transform::identity);
        let mut blue_flag_tf = world
            .get::<Transform>(refs.blue_flag)
            .cloned()
            .unwrap_or_else(Transform::identity);
        let midline_x = world
            .resource::<MapSize>()
            .map(|map_size| map_size.midline_x())
            .unwrap_or(crate::game::ctf::MIDLINE_X);

        let mut dirty = knock_down_moving_flags(
            &player_transforms,
            &red_flag_tf,
            &blue_flag_tf,
            &mut flag_motion,
        );

        let mut sync = CtfSyncState::dirty();
        for tagger in CtfSlot::ALL {
            if let Some(tagged) = tag_nearest_opponent(
                tagger,
                &refs,
                &mut carrier,
                &mut player_transforms,
                &mut velocities,
                &mut red_flag_tf,
                &mut blue_flag_tf,
                midline_x,
            ) {
                dirty = true;
                sync.force_player(tagged);
            }
        }

        if !dirty {
            return;
        }

        for slot in CtfSlot::ALL {
            commands.insert(refs.player(slot), player_transforms[slot.index()].clone());
            commands.insert(refs.player(slot), velocities[slot.index()].clone());
        }
        commands.insert(refs.red_flag, red_flag_tf);
        commands.insert(refs.blue_flag, blue_flag_tf);
        commands.insert_resource(carrier);
        commands.insert_resource(flag_motion);
        commands.insert_resource(sync);
    }
}

fn knock_down_moving_flags(
    player_transforms: &[Transform; CtfSlot::COUNT],
    red_flag_tf: &Transform,
    blue_flag_tf: &Transform,
    flag_motion: &mut FlagMotionState,
) -> bool {
    let mut dirty = false;
    let range_sq = FLAG_KNOCKDOWN_RANGE * FLAG_KNOCKDOWN_RANGE;

    if let Some(motion) = flag_motion.red {
        if CtfSlot::ALL.iter().any(|slot| {
            Some(*slot) != motion.released_by
                && slot_in_range(*slot, player_transforms, red_flag_tf, range_sq)
        }) {
            flag_motion.red = None;
            dirty = true;
        }
    }

    if let Some(motion) = flag_motion.blue {
        if CtfSlot::ALL.iter().any(|slot| {
            Some(*slot) != motion.released_by
                && slot_in_range(*slot, player_transforms, blue_flag_tf, range_sq)
        }) {
            flag_motion.blue = None;
            dirty = true;
        }
    }

    dirty
}

fn slot_in_range(
    slot: CtfSlot,
    player_transforms: &[Transform; CtfSlot::COUNT],
    target: &Transform,
    range_sq: f32,
) -> bool {
    let player = &player_transforms[slot.index()];
    distance_sq(
        player.position.x,
        player.position.y,
        target.position.x,
        target.position.y,
    ) <= range_sq
}

fn tag_nearest_opponent(
    tagger: CtfSlot,
    refs: &EntityRefs,
    carrier: &mut CarrierState,
    player_transforms: &mut [Transform; CtfSlot::COUNT],
    velocities: &mut [Velocity; CtfSlot::COUNT],
    red_flag_tf: &mut Transform,
    blue_flag_tf: &mut Transform,
    midline_x: f32,
) -> Option<CtfSlot> {
    if carries_any_flag(tagger, carrier) {
        return None;
    }

    let tagger_tf = &player_transforms[tagger.index()];
    let tagger_x = tagger_tf.position.x;
    let tagger_y = tagger_tf.position.y;
    let range_sq = TAG_RANGE * TAG_RANGE;

    let mut best: Option<(CtfSlot, f32)> = None;
    for opponent in tagger.opponent_slots() {
        if !can_tag(tagger, *opponent, player_transforms, midline_x) {
            continue;
        }

        let opponent_tf = &player_transforms[opponent.index()];
        let dist = distance_sq(
            tagger_x,
            tagger_y,
            opponent_tf.position.x,
            opponent_tf.position.y,
        );
        if dist > range_sq {
            continue;
        }

        match best {
            Some((best_slot, best_dist))
                if dist > best_dist
                    || (dist == best_dist && opponent.index() > best_slot.index()) => {}
            _ => best = Some((*opponent, dist)),
        }
    }

    let Some((tagged, _)) = best else {
        return None;
    };

    drop_carried_flags(
        tagged,
        carrier,
        player_transforms,
        red_flag_tf,
        blue_flag_tf,
    );

    let spawn = refs.player_spawn(tagged);
    player_transforms[tagged.index()].position = Vec3::new(
        spawn.0,
        spawn.1,
        player_transforms[tagged.index()].position.z,
    );
    velocities[tagged.index()] = Velocity { dx: 0.0, dy: 0.0 };
    Some(tagged)
}

fn drop_carried_flags(
    tagged: CtfSlot,
    carrier: &mut CarrierState,
    player_transforms: &[Transform; CtfSlot::COUNT],
    red_flag_tf: &mut Transform,
    blue_flag_tf: &mut Transform,
) {
    let tagged_position = player_transforms[tagged.index()].position;
    if carrier.red_flag_carrier == Some(tagged) {
        carrier.red_flag_carrier = None;
        red_flag_tf.position =
            Vec3::new(tagged_position.x, tagged_position.y, red_flag_tf.position.z);
    }
    if carrier.blue_flag_carrier == Some(tagged) {
        carrier.blue_flag_carrier = None;
        blue_flag_tf.position = Vec3::new(
            tagged_position.x,
            tagged_position.y,
            blue_flag_tf.position.z,
        );
    }
}

fn can_tag(
    tagger: CtfSlot,
    opponent: CtfSlot,
    player_transforms: &[Transform; CtfSlot::COUNT],
    midline_x: f32,
) -> bool {
    let tagger_x = player_transforms[tagger.index()].position.x;
    let opponent_x = player_transforms[opponent.index()].position.x;
    is_on_own_side(tagger, tagger_x, midline_x) && is_on_own_side(tagger, opponent_x, midline_x)
}

fn is_on_own_side(slot: CtfSlot, x: f32, midline_x: f32) -> bool {
    if slot.is_red() {
        x < midline_x
    } else {
        x > midline_x
    }
}

fn carries_any_flag(slot: CtfSlot, carrier: &CarrierState) -> bool {
    carrier.red_flag_carrier == Some(slot) || carrier.blue_flag_carrier == Some(slot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::command_buffer::CommandBuffer;
    use crate::game::ctf::setup::setup_ctf_scene_entities;
    use crate::game::ctf::systems::FlagPickupSystem;
    use crate::multiplayer::matchmaking::CtfSlotAssignment;
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

    fn run_tagging(world: &mut World) {
        let mut commands = CommandBuffer::new();
        TaggingSystem.run(world, &mut commands);
        commands.flush(world);
    }

    fn run_pickup(world: &mut World) {
        let mut commands = CommandBuffer::new();
        FlagPickupSystem.run(world, &mut commands);
        commands.flush(world);
    }

    #[test]
    fn moving_own_flag_is_not_knocked_down_by_release_slot() {
        let mut world = test_world();
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
        let mut red_flag_tf = world
            .get::<Transform>(refs.red_flag)
            .cloned()
            .expect("red flag");
        red_flag_tf.position.x = 320.0;
        red_flag_tf.position.y = 300.0;
        world.insert(refs.red_flag, red_flag_tf);
        world.insert_resource(FlagMotionState {
            red: Some(crate::game::ctf::resources::FlagMotion {
                dir_x: 1.0,
                dir_y: 0.0,
                remaining_distance: 80.0,
                released_by: Some(CtfSlot::Red1),
            }),
            blue: None,
        });

        run_tagging(&mut world);

        let motion = world.resource::<FlagMotionState>().expect("flag motion");
        assert!(motion.red.is_some());
    }

    #[test]
    fn moving_own_flag_can_still_be_knocked_down_by_teammate() {
        let mut world = test_world();
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
        let red3 = refs.player(CtfSlot::Red3);
        let mut red3_tf = world.get::<Transform>(red3).cloned().expect("red3");
        red3_tf.position.x = 340.0;
        red3_tf.position.y = 300.0;
        world.insert(red3, red3_tf);
        let mut red_flag_tf = world
            .get::<Transform>(refs.red_flag)
            .cloned()
            .expect("red flag");
        red_flag_tf.position.x = 320.0;
        red_flag_tf.position.y = 300.0;
        world.insert(refs.red_flag, red_flag_tf);
        world.insert_resource(FlagMotionState {
            red: Some(crate::game::ctf::resources::FlagMotion {
                dir_x: 1.0,
                dir_y: 0.0,
                remaining_distance: 80.0,
                released_by: Some(CtfSlot::Red1),
            }),
            blue: None,
        });

        run_tagging(&mut world);

        let motion = world.resource::<FlagMotionState>().expect("flag motion");
        assert_eq!(motion.red, None);
    }

    #[test]
    fn moving_opponent_flag_can_be_knocked_down_and_picked_up() {
        let mut world = test_world();
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
        let blue = refs.player(CtfSlot::Blue1);
        let blue_tf = world.get::<Transform>(blue).cloned().expect("blue");
        let mut red_flag_tf = world
            .get::<Transform>(refs.red_flag)
            .cloned()
            .expect("red flag");
        red_flag_tf.position.x = blue_tf.position.x;
        red_flag_tf.position.y = blue_tf.position.y;
        world.insert(refs.red_flag, red_flag_tf);
        world.insert_resource(FlagMotionState {
            red: Some(crate::game::ctf::resources::FlagMotion {
                dir_x: 1.0,
                dir_y: 0.0,
                remaining_distance: 80.0,
                released_by: Some(CtfSlot::Red1),
            }),
            blue: None,
        });

        run_tagging(&mut world);
        run_pickup(&mut world);

        let motion = world.resource::<FlagMotionState>().expect("flag motion");
        assert_eq!(motion.red, None);
        let carrier = world.resource::<CarrierState>().expect("carrier");
        assert_eq!(carrier.red_flag_carrier, Some(CtfSlot::Blue1));
    }

    #[test]
    fn own_flag_carrier_cannot_tag_opponents() {
        let mut world = test_world();
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
        let blue = refs.player(CtfSlot::Blue1);
        let mut blue_tf = world.get::<Transform>(blue).cloned().expect("blue");
        blue_tf.position.x = 320.0;
        blue_tf.position.y = 300.0;
        world.insert(blue, blue_tf);
        world.insert_resource(CarrierState {
            red_flag_carrier: Some(CtfSlot::Red1),
            blue_flag_carrier: None,
        });

        run_tagging(&mut world);

        let blue_tf = world.get::<Transform>(blue).expect("blue");
        assert_eq!((blue_tf.position.x, blue_tf.position.y), (320.0, 300.0));
    }

    #[test]
    fn enemy_flag_carrier_cannot_tag_opponents() {
        let mut world = test_world();
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
        let blue = refs.player(CtfSlot::Blue1);
        let mut blue_tf = world.get::<Transform>(blue).cloned().expect("blue");
        blue_tf.position.x = 320.0;
        blue_tf.position.y = 300.0;
        world.insert(blue, blue_tf);
        world.insert_resource(CarrierState {
            red_flag_carrier: None,
            blue_flag_carrier: Some(CtfSlot::Red1),
        });

        run_tagging(&mut world);

        let blue_tf = world.get::<Transform>(blue).expect("blue");
        assert_eq!((blue_tf.position.x, blue_tf.position.y), (320.0, 300.0));
    }

    #[test]
    fn own_flag_carrier_in_red_territory_cannot_be_tagged_by_enemy_in_red_territory() {
        let mut world = test_world();
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
        let red = refs.player(CtfSlot::Red1);
        let blue = refs.player(CtfSlot::Blue1);
        let mut red_tf = world.get::<Transform>(red).cloned().expect("red");
        red_tf.position.x = 320.0;
        red_tf.position.y = 300.0;
        world.insert(red, red_tf);
        let mut blue_tf = world.get::<Transform>(blue).cloned().expect("blue");
        blue_tf.position.x = 340.0;
        blue_tf.position.y = 300.0;
        world.insert(blue, blue_tf);
        world.insert_resource(CarrierState {
            red_flag_carrier: Some(CtfSlot::Red1),
            blue_flag_carrier: None,
        });

        run_tagging(&mut world);

        // Blue is in Red territory (not own territory), so cannot tag.
        let red_tf = world.get::<Transform>(red).expect("red");
        assert_eq!((red_tf.position.x, red_tf.position.y), (320.0, 300.0));
        let carrier = world.resource::<CarrierState>().expect("carrier");
        assert_eq!(carrier.red_flag_carrier, Some(CtfSlot::Red1));
    }

    #[test]
    fn own_flag_carrier_in_blue_territory_can_be_tagged_by_enemy_in_blue_territory() {
        let mut world = test_world();
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
        let red = refs.player(CtfSlot::Red1);
        let blue = refs.player(CtfSlot::Blue1);
        let mut red_tf = world.get::<Transform>(red).cloned().expect("red");
        red_tf.position.x = 700.0;
        red_tf.position.y = 300.0;
        world.insert(red, red_tf);
        let mut blue_tf = world.get::<Transform>(blue).cloned().expect("blue");
        blue_tf.position.x = 720.0;
        blue_tf.position.y = 300.0;
        world.insert(blue, blue_tf);
        world.insert_resource(CarrierState {
            red_flag_carrier: Some(CtfSlot::Red1),
            blue_flag_carrier: None,
        });

        run_tagging(&mut world);

        let red_tf = world.get::<Transform>(red).expect("red");
        assert_eq!(
            (red_tf.position.x, red_tf.position.y),
            refs.player_spawn(CtfSlot::Red1)
        );
        let carrier = world.resource::<CarrierState>().expect("carrier");
        assert_eq!(carrier.red_flag_carrier, None);
        let red_flag = world.get::<Transform>(refs.red_flag).expect("red flag");
        assert_eq!((red_flag.position.x, red_flag.position.y), (700.0, 300.0));
    }
}
