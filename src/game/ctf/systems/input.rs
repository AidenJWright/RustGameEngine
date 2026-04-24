//! Network/local input application for CTF.

use crate::components::{Transform, Velocity};
use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::system::System;
use crate::ecs::world::World;
use crate::game::ctf::nav::find_path;
use crate::game::ctf::resources::{
    AutoMovePath, AutoMoveState, CarrierState, ControlState, CtfInputState, CtfPointerState,
    CtfSyncState, EntityRefs, FlagMotion, FlagMotionState, GameState, NavigationGrid,
};
use crate::game::ctf::{
    ACTION_RESTART, ACTION_SWITCH, ACTION_TAG, FLAG_KNOCKDOWN_RANGE, FLAG_THROW_DISTANCE,
    MIDLINE_X, PLAYER_SPEED, TAG_RANGE,
};
use crate::math::Vec3;
use crate::multiplayer::matchmaking::CtfSlot;
use crate::multiplayer::InputFrame;

use super::{distance_sq, is_playing};

/// Applies CTF input frames to selected scene-authored player slots.
#[derive(Debug, Default)]
pub struct CtfInputSystem;

impl System for CtfInputSystem {
    fn run(&self, world: &World, commands: &mut CommandBuffer) {
        let mut frames = world
            .resource::<CtfInputState>()
            .cloned()
            .unwrap_or_default()
            .frames;
        frames.sort_by_key(|frame| (frame.tick, frame.player_id));
        commands.insert_resource(CtfInputState::default());

        // When the frame rate exceeds the tick rate, some frames contain no
        // input.  Returning here lets the last tick's velocity persist so
        // keyboard-controlled entities keep moving at the same effective speed
        // as AutoMoveSystem entities, which write velocity every frame.
        if frames.is_empty() {
            return;
        }

        let Some(refs) = world.resource::<EntityRefs>().cloned() else {
            return;
        };

        let Some(mut controls) = world.resource::<ControlState>().cloned() else {
            return;
        };
        let (restart_requested, restart_key_changed) =
            update_restart_state(&frames, &mut controls);
        if restart_requested {
            reset_ctf_match(world, commands, &refs, controls);
            return;
        }

        if !is_playing(world) {
            for slot in CtfSlot::ALL {
                commands.insert(refs.player(slot), Velocity { dx: 0.0, dy: 0.0 });
            }
            if restart_key_changed {
                commands.insert_resource(controls);
            }
            return;
        }

        let mut carrier = world.resource::<CarrierState>().copied().unwrap_or_default();
        let original_carrier = carrier;
        let mut flag_motion = world.resource::<FlagMotionState>().copied().unwrap_or_default();
        let original_flag_motion = flag_motion;
        let mut auto_move = world.resource::<AutoMoveState>().cloned().unwrap_or_default();
        let original_auto_move = auto_move.clone();
        let nav_grid = world.resource::<NavigationGrid>().cloned();

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
                &mut flag_motion,
                &mut auto_move,
                nav_grid.as_ref(),
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
        if flag_motion != original_flag_motion {
            commands.insert_resource(flag_motion);
            dirty = true;
        }
        if auto_move != original_auto_move {
            commands.insert_resource(auto_move);
            dirty = true;
        }
        if dirty {
            commands.insert_resource(CtfSyncState { dirty: true });
        }
    }
}

fn update_restart_state(frames: &[InputFrame], controls: &mut ControlState) -> (bool, bool) {
    let mut restart_requested = false;
    let mut changed = false;
    for frame in frames {
        let Some(control) = controls
            .controls
            .iter_mut()
            .find(|control| control.client_id == frame.player_id)
        else {
            continue;
        };
        let restart_down = frame.action_bits & ACTION_RESTART != 0;
        restart_requested |= restart_down && !control.restart_down;
        changed |= restart_down != control.restart_down;
        control.restart_down = restart_down;
    }
    (restart_requested, changed)
}

fn reset_ctf_match(
    world: &World,
    commands: &mut CommandBuffer,
    refs: &EntityRefs,
    mut controls: ControlState,
) {
    for control in &mut controls.controls {
        control.selected_slot = control.primary_slot;
        control.tag_down = false;
        control.switch_down = false;
    }

    for slot in CtfSlot::ALL {
        let entity = refs.player(slot);
        if let Some(transform) = world.get::<Transform>(entity) {
            let spawn = refs.player_spawn(slot);
            let mut next = transform.clone();
            next.position = Vec3::new(spawn.0, spawn.1, transform.position.z);
            commands.insert(entity, next);
        }
        commands.insert(entity, Velocity { dx: 0.0, dy: 0.0 });
    }

    if let Some(transform) = world.get::<Transform>(refs.red_flag) {
        let mut next = transform.clone();
        next.position = Vec3::new(refs.red_flag_spawn.0, refs.red_flag_spawn.1, transform.position.z);
        commands.insert(refs.red_flag, next);
    }
    if let Some(transform) = world.get::<Transform>(refs.blue_flag) {
        let mut next = transform.clone();
        next.position = Vec3::new(
            refs.blue_flag_spawn.0,
            refs.blue_flag_spawn.1,
            transform.position.z,
        );
        commands.insert(refs.blue_flag, next);
    }

    if let Some(mut pointer) = world.resource::<CtfPointerState>().copied() {
        pointer.pending_left_click_world = None;
        commands.insert_resource(pointer);
    }
    commands.insert_resource(GameState::default());
    commands.insert_resource(CarrierState::default());
    commands.insert_resource(FlagMotionState::default());
    commands.insert_resource(AutoMoveState::default());
    commands.insert_resource(CtfInputState::default());
    commands.insert_resource(controls);
    commands.insert_resource(CtfSyncState { dirty: true });
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
    flag_motion: &mut FlagMotionState,
    auto_move: &mut AutoMoveState,
    nav_grid: Option<&NavigationGrid>,
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

    let selected_slot = control.selected_slot;
    let has_keyboard = frame.move_x.abs() > f32::EPSILON || frame.move_y.abs() > f32::EPSILON;

    if has_keyboard {
        if auto_move.paths[selected_slot.index()].take().is_some() {
            dirty = true;
        }
    } else if let Some(click_world) = frame
        .ctf_pointer
        .as_ref()
        .and_then(|pointer| pointer.click_world)
    {
        if let Some(grid) = nav_grid {
            let start = (
                player_transforms[selected_slot.index()].position.x,
                player_transforms[selected_slot.index()].position.y,
            );
            match find_path(grid, start, click_world) {
                Some(waypoints) if !waypoints.is_empty() => {
                    auto_move.paths[selected_slot.index()] = Some(AutoMovePath {
                        waypoints,
                        next_index: 0,
                    });
                    dirty = true;
                }
                _ => {
                    if auto_move.paths[selected_slot.index()].take().is_some() {
                        dirty = true;
                    }
                }
            }
        }
    }

    if tag_pressed {
        let acted = throw_carried_flag(
            selected_slot,
            frame.ctf_pointer.as_ref().and_then(|pointer| pointer.aim_world),
            carrier,
            player_transforms,
            red_flag_tf,
            blue_flag_tf,
            flag_motion,
        ) || knock_down_own_moving_flag(
            selected_slot,
            player_transforms,
            red_flag_tf,
            blue_flag_tf,
            flag_motion,
        ) || tag_nearest_opponent(
            selected_slot,
            refs,
            carrier,
            player_transforms,
            velocities,
            red_flag_tf,
            blue_flag_tf,
        );
        if acted {
            dirty = true;
        }
    }

    velocities[selected_slot.index()] = Velocity {
        dx: frame.move_x * PLAYER_SPEED,
        dy: frame.move_y * PLAYER_SPEED,
    };
    control.tag_down = tag_down;
    control.switch_down = switch_down;
    dirty
}

fn throw_carried_flag(
    slot: CtfSlot,
    aim_world: Option<(f32, f32)>,
    carrier: &mut CarrierState,
    player_transforms: &[Transform; 4],
    red_flag_tf: &mut Transform,
    blue_flag_tf: &mut Transform,
    flag_motion: &mut FlagMotionState,
) -> bool {
    let throws_blue = slot.is_red() && carrier.blue_flag_carrier == Some(slot);
    let throws_red = !slot.is_red() && carrier.red_flag_carrier == Some(slot);
    if !throws_blue && !throws_red {
        return false;
    }

    let player_pos = player_transforms[slot.index()].position;
    let (dir_x, dir_y) = throw_direction(slot, player_pos.x, player_pos.y, aim_world);
    let motion = Some(FlagMotion {
        dir_x,
        dir_y,
        remaining_distance: FLAG_THROW_DISTANCE,
    });

    if throws_blue {
        carrier.blue_flag_carrier = None;
        blue_flag_tf.position = Vec3::new(player_pos.x, player_pos.y, blue_flag_tf.position.z);
        flag_motion.blue = motion;
    } else {
        carrier.red_flag_carrier = None;
        red_flag_tf.position = Vec3::new(player_pos.x, player_pos.y, red_flag_tf.position.z);
        flag_motion.red = motion;
    }

    true
}

fn throw_direction(
    slot: CtfSlot,
    player_x: f32,
    player_y: f32,
    aim_world: Option<(f32, f32)>,
) -> (f32, f32) {
    if let Some((aim_x, aim_y)) = aim_world {
        let dx = aim_x - player_x;
        let dy = aim_y - player_y;
        let len = (dx * dx + dy * dy).sqrt();
        if len > f32::EPSILON {
            return (dx / len, dy / len);
        }
    }

    if slot.is_red() {
        (1.0, 0.0)
    } else {
        (-1.0, 0.0)
    }
}

fn knock_down_own_moving_flag(
    slot: CtfSlot,
    player_transforms: &[Transform; 4],
    red_flag_tf: &Transform,
    blue_flag_tf: &Transform,
    flag_motion: &mut FlagMotionState,
) -> bool {
    let player = &player_transforms[slot.index()];
    let range_sq = FLAG_KNOCKDOWN_RANGE * FLAG_KNOCKDOWN_RANGE;
    if slot.is_red() {
        if flag_motion.red.is_none() {
            return false;
        }
        let dist = distance_sq(
            player.position.x,
            player.position.y,
            red_flag_tf.position.x,
            red_flag_tf.position.y,
        );
        if dist <= range_sq {
            flag_motion.red = None;
            return true;
        }
    } else {
        if flag_motion.blue.is_none() {
            return false;
        }
        let dist = distance_sq(
            player.position.x,
            player.position.y,
            blue_flag_tf.position.x,
            blue_flag_tf.position.y,
        );
        if dist <= range_sq {
            flag_motion.blue = None;
            return true;
        }
    }
    false
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
    use crate::game::ctf::resources::{
        AutoMovePath, AutoMoveState, FlagMotion, FlagMotionState, GamePhase, GameState,
    };
    use crate::game::ctf::setup::setup_ctf_scene_entities;
    use crate::multiplayer::matchmaking::CtfSlotAssignment;
    use crate::multiplayer::CtfPointerInput;
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
                ctf_pointer: None,
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
                ctf_pointer: None,
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
                ctf_pointer: None,
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

    #[test]
    fn carrier_action_throws_flag_toward_cursor() {
        let mut world = test_world();
        world.insert_resource(CarrierState {
            red_flag_carrier: None,
            blue_flag_carrier: Some(CtfSlot::Red1),
        });
        world.insert_resource(CtfInputState {
            frames: vec![InputFrame {
                tick: 1,
                player_id: 1,
                move_x: 0.0,
                move_y: 0.0,
                action_bits: ACTION_TAG,
                ctf_pointer: Some(CtfPointerInput {
                    aim_world: Some((260.0, 320.0)),
                    click_world: None,
                }),
            }],
        });

        let mut commands = CommandBuffer::new();
        CtfInputSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        let carrier = world.resource::<CarrierState>().expect("carrier");
        assert_eq!(carrier.blue_flag_carrier, None);
        let motion = world.resource::<FlagMotionState>().expect("flag motion");
        assert_eq!(
            motion.blue,
            Some(FlagMotion {
                dir_x: 1.0,
                dir_y: 0.0,
                remaining_distance: crate::game::ctf::FLAG_THROW_DISTANCE,
            })
        );
    }

    #[test]
    fn owner_action_knocks_down_nearby_moving_flag() {
        let mut world = test_world();
        world.insert_resource(FlagMotionState {
            red: Some(FlagMotion {
                dir_x: 1.0,
                dir_y: 0.0,
                remaining_distance: 80.0,
            }),
            blue: None,
        });
        world.insert_resource(CtfInputState {
            frames: vec![InputFrame {
                tick: 1,
                player_id: 1,
                move_x: 0.0,
                move_y: 0.0,
                action_bits: ACTION_TAG,
                ctf_pointer: None,
            }],
        });

        let mut commands = CommandBuffer::new();
        CtfInputSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        let motion = world.resource::<FlagMotionState>().expect("flag motion");
        assert_eq!(motion.red, None);
    }

    #[test]
    fn click_sets_path_for_selected_slot() {
        let mut world = test_world();
        world.insert_resource(CtfInputState {
            frames: vec![InputFrame {
                tick: 1,
                player_id: 1,
                move_x: 0.0,
                move_y: 0.0,
                action_bits: 0,
                ctf_pointer: Some(CtfPointerInput {
                    aim_world: Some((260.0, 320.0)),
                    click_world: Some((260.0, 320.0)),
                }),
            }],
        });

        let mut commands = CommandBuffer::new();
        CtfInputSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        let auto_move = world.resource::<AutoMoveState>().expect("auto move");
        assert!(auto_move.paths[CtfSlot::Red1.index()]
            .as_ref()
            .is_some_and(|path| !path.waypoints.is_empty()));
    }

    #[test]
    fn restart_action_resets_ctf_match_state() {
        let mut world = test_world();
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
        let red = refs.player(CtfSlot::Red1);
        let mut red_tf = world.get::<Transform>(red).cloned().expect("red tf");
        red_tf.position.x = 500.0;
        red_tf.position.y = 500.0;
        world.insert(red, red_tf);
        world.insert(red, Velocity { dx: 123.0, dy: 45.0 });
        let mut blue_flag_tf = world
            .get::<Transform>(refs.blue_flag)
            .cloned()
            .expect("blue flag");
        blue_flag_tf.position.x = 300.0;
        blue_flag_tf.position.y = 300.0;
        world.insert(refs.blue_flag, blue_flag_tf);
        world.insert_resource(GameState {
            phase: GamePhase::Won(1),
        });
        world.insert_resource(CarrierState {
            red_flag_carrier: None,
            blue_flag_carrier: Some(CtfSlot::Red1),
        });
        world.insert_resource(FlagMotionState {
            red: None,
            blue: Some(FlagMotion {
                dir_x: 1.0,
                dir_y: 0.0,
                remaining_distance: 80.0,
            }),
        });
        let mut paths = std::array::from_fn(|_| None);
        paths[CtfSlot::Red1.index()] = Some(AutoMovePath {
            waypoints: vec![(400.0, 400.0)],
            next_index: 0,
        });
        world.insert_resource(AutoMoveState { paths });
        world.insert_resource(CtfInputState {
            frames: vec![InputFrame {
                tick: 1,
                player_id: 1,
                move_x: 0.0,
                move_y: 0.0,
                action_bits: ACTION_RESTART,
                ctf_pointer: None,
            }],
        });

        let mut commands = CommandBuffer::new();
        CtfInputSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        assert!(matches!(
            world.resource::<GameState>().expect("game state").phase,
            GamePhase::Playing
        ));
        assert_eq!(world.resource::<CarrierState>().expect("carrier"), &CarrierState::default());
        assert_eq!(
            *world.resource::<FlagMotionState>().expect("flag motion"),
            FlagMotionState::default()
        );
        assert_eq!(
            *world.resource::<AutoMoveState>().expect("auto move"),
            AutoMoveState::default()
        );
        let red_tf = world.get::<Transform>(red).expect("red tf");
        assert_eq!((red_tf.position.x, red_tf.position.y), refs.player_spawn(CtfSlot::Red1));
        let red_vel = world.get::<Velocity>(red).expect("red velocity");
        assert_eq!((red_vel.dx, red_vel.dy), (0.0, 0.0));
        let blue_flag_tf = world.get::<Transform>(refs.blue_flag).expect("blue flag");
        assert_eq!(
            (blue_flag_tf.position.x, blue_flag_tf.position.y),
            refs.blue_flag_spawn
        );
    }
}
