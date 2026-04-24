//! Network/local input application for CTF.

use crate::components::{Transform, Velocity};
use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::system::System;
use crate::ecs::world::World;
use crate::game::ctf::resources::{
    CarrierState, ControlState, CtfInputState, CtfSyncState, EntityRefs,
};
use crate::game::ctf::{ACTION_SWITCH, ACTION_TAG, MIDLINE_X, PLAYER_SPEED, TAG_RANGE};
use crate::math::Vec3;
use crate::multiplayer::matchmaking::CtfSlot;
use crate::multiplayer::InputFrame;

use super::{distance_sq, is_playing};

/// Applies CTF input frames to selected scene-authored player slots.
#[derive(Debug, Default)]
pub struct CtfInputSystem;

impl System for CtfInputSystem {
    fn run(&self, world: &World, commands: &mut CommandBuffer) {
        let frames = world
            .resource::<CtfInputState>()
            .cloned()
            .unwrap_or_default()
            .frames;
        commands.insert_resource(CtfInputState::default());

        let Some(refs) = world.resource::<EntityRefs>().cloned() else {
            return;
        };

        if !is_playing(world) {
            for slot in CtfSlot::ALL {
                commands.insert(refs.player(slot), Velocity { dx: 0.0, dy: 0.0 });
            }
            return;
        }

        let Some(mut controls) = world.resource::<ControlState>().cloned() else {
            return;
        };
        let mut carrier = world.resource::<CarrierState>().copied().unwrap_or_default();
        let original_carrier = carrier;

        let mut player_transforms = std::array::from_fn(|idx| {
            world
                .get::<Transform>(refs.players[idx])
                .cloned()
                .unwrap_or_else(Transform::identity)
        });
        let mut velocities =
            std::array::from_fn(|_| Velocity { dx: 0.0, dy: 0.0 });
        let mut red_flag_tf = world
            .get::<Transform>(refs.red_flag)
            .cloned()
            .unwrap_or_else(Transform::identity);
        let mut blue_flag_tf = world
            .get::<Transform>(refs.blue_flag)
            .cloned()
            .unwrap_or_else(Transform::identity);

        let mut dirty = false;

        for frame in frames {
            if apply_frame(
                &frame,
                &refs,
                &mut controls,
                &mut carrier,
                &mut player_transforms,
                &mut velocities,
                &mut red_flag_tf,
                &mut blue_flag_tf,
            ) {
                dirty = true;
            }
        }

        for slot in CtfSlot::ALL {
            commands.insert(refs.player(slot), player_transforms[slot.index()].clone());
            commands.insert(refs.player(slot), velocities[slot.index()].clone());
        }
        commands.insert(refs.red_flag, red_flag_tf);
        commands.insert(refs.blue_flag, blue_flag_tf);
        commands.insert_resource(controls);
        if carrier.red_flag_carrier != original_carrier.red_flag_carrier
            || carrier.blue_flag_carrier != original_carrier.blue_flag_carrier
        {
            dirty = true;
        }
        commands.insert_resource(carrier);
        if dirty {
            commands.insert_resource(CtfSyncState { dirty: true });
        }
    }
}

fn apply_frame(
    frame: &InputFrame,
    refs: &EntityRefs,
    controls: &mut ControlState,
    carrier: &mut CarrierState,
    player_transforms: &mut [Transform; 4],
    velocities: &mut [Velocity; 4],
    red_flag_tf: &mut Transform,
    blue_flag_tf: &mut Transform,
) -> bool {
    let Some(control) = controls
        .controls
        .iter_mut()
        .find(|control| control.client_id == frame.player_id)
    else {
        return false;
    };

    let tag_down = frame.action_bits & ACTION_TAG != 0;
    let switch_down = frame.action_bits & ACTION_SWITCH != 0;
    let tag_pressed = tag_down && !control.tag_down;
    let switch_pressed = switch_down && !control.switch_down;
    let mut dirty = false;

    if switch_pressed && control.owned_slots.len() > 1 {
        let current_index = control
            .owned_slots
            .iter()
            .position(|slot| *slot == control.selected_slot)
            .unwrap_or(0);
        control.selected_slot = control.owned_slots[(current_index + 1) % control.owned_slots.len()];
        dirty = true;
    }

    if tag_pressed
        && tag_nearest_opponent(
            control.selected_slot,
            refs,
            carrier,
            player_transforms,
            velocities,
            red_flag_tf,
            blue_flag_tf,
        )
    {
        dirty = true;
    }

    velocities[control.selected_slot.index()] = Velocity {
        dx: frame.move_x * PLAYER_SPEED,
        dy: frame.move_y * PLAYER_SPEED,
    };
    control.tag_down = tag_down;
    control.switch_down = switch_down;
    dirty
}

fn tag_nearest_opponent(
    tagger: CtfSlot,
    refs: &EntityRefs,
    carrier: &mut CarrierState,
    player_transforms: &mut [Transform; 4],
    velocities: &mut [Velocity; 4],
    red_flag_tf: &mut Transform,
    blue_flag_tf: &mut Transform,
) -> bool {
    let tagger_tf = &player_transforms[tagger.index()];
    let tagger_x = tagger_tf.position.x;
    let tagger_y = tagger_tf.position.y;
    let range_sq = TAG_RANGE * TAG_RANGE;

    let mut best: Option<(CtfSlot, f32)> = None;
    for opponent in tagger.opponent_slots() {
        let opponent_tf = &player_transforms[opponent.index()];
        let opponent_on_defender_side = if tagger.is_red() {
            opponent_tf.position.x < MIDLINE_X
        } else {
            opponent_tf.position.x > MIDLINE_X
        };
        if !opponent_on_defender_side {
            continue;
        }

        let dist = distance_sq(
            tagger_x,
            tagger_y,
            opponent_tf.position.x,
            opponent_tf.position.y,
        );
        if dist >= range_sq {
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
        return false;
    };

    let tagged_position = player_transforms[tagged.index()].position;
    if carrier.red_flag_carrier == Some(tagged) {
        carrier.red_flag_carrier = None;
        red_flag_tf.position = Vec3::new(tagged_position.x, tagged_position.y, red_flag_tf.position.z);
    }
    if carrier.blue_flag_carrier == Some(tagged) {
        carrier.blue_flag_carrier = None;
        blue_flag_tf.position =
            Vec3::new(tagged_position.x, tagged_position.y, blue_flag_tf.position.z);
    }

    let spawn = refs.player_spawn(tagged);
    player_transforms[tagged.index()].position = Vec3::new(
        spawn.0,
        spawn.1,
        player_transforms[tagged.index()].position.z,
    );
    velocities[tagged.index()] = Velocity { dx: 0.0, dy: 0.0 };
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::command_buffer::CommandBuffer;
    use crate::game::ctf::setup::setup_ctf_scene_entities;
    use crate::multiplayer::matchmaking::CtfSlotAssignment;
    use crate::scene::reload_scene;

    fn test_world() -> World {
        let mut world = World::new();
        reload_scene(&mut world, "assets/ctf_scene.json").expect("load CTF scene");
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
        );
        world
    }

    #[test]
    fn left_shift_switches_only_owned_slots() {
        let mut world = test_world();
        world.insert_resource(CtfInputState {
            frames: vec![InputFrame {
                tick: 1,
                player_id: 1,
                move_x: 0.0,
                move_y: 0.0,
                action_bits: ACTION_SWITCH,
            }],
        });

        let mut commands = CommandBuffer::new();
        CtfInputSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        let controls = world.resource::<ControlState>().expect("controls");
        let red = controls
            .controls
            .iter()
            .find(|control| control.client_id == 1)
            .expect("red control");
        let blue = controls
            .controls
            .iter()
            .find(|control| control.client_id == 2)
            .expect("blue control");
        assert_eq!(red.selected_slot, CtfSlot::Red2);
        assert_eq!(blue.selected_slot, CtfSlot::Blue1);
    }

    #[test]
    fn tag_teleports_non_carrier_opponent_home() {
        let mut world = test_world();
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
        let blue = refs.player(CtfSlot::Blue1);
        let mut blue_tf = world.get::<Transform>(blue).cloned().expect("blue tf");
        blue_tf.position.x = 180.0;
        blue_tf.position.y = 320.0;
        world.insert(blue, blue_tf);
        world.insert_resource(CtfInputState {
            frames: vec![InputFrame {
                tick: 1,
                player_id: 1,
                move_x: 0.0,
                move_y: 0.0,
                action_bits: ACTION_TAG,
            }],
        });

        let mut commands = CommandBuffer::new();
        CtfInputSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        let blue_tf = world.get::<Transform>(blue).expect("blue tf");
        let spawn = refs.player_spawn(CtfSlot::Blue1);
        assert_eq!((blue_tf.position.x, blue_tf.position.y), spawn);
    }

    #[test]
    fn tag_drops_carried_flag_at_pre_teleport_position() {
        let mut world = test_world();
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
        let blue = refs.player(CtfSlot::Blue1);
        let mut blue_tf = world.get::<Transform>(blue).cloned().expect("blue tf");
        blue_tf.position.x = 180.0;
        blue_tf.position.y = 320.0;
        world.insert(blue, blue_tf);
        world.insert_resource(CarrierState {
            red_flag_carrier: Some(CtfSlot::Blue1),
            blue_flag_carrier: None,
        });
        world.insert_resource(CtfInputState {
            frames: vec![InputFrame {
                tick: 1,
                player_id: 1,
                move_x: 0.0,
                move_y: 0.0,
                action_bits: ACTION_TAG,
            }],
        });

        let mut commands = CommandBuffer::new();
        CtfInputSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        let carrier = world.resource::<CarrierState>().expect("carrier");
        assert_eq!(carrier.red_flag_carrier, None);
        let red_flag = world
            .get::<Transform>(refs.red_flag)
            .expect("red flag transform");
        assert_eq!((red_flag.position.x, red_flag.position.y), (180.0, 320.0));
    }
}
