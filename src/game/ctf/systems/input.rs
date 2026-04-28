//! Network/local input application for CTF.

use crate::components::{Transform, Velocity};
use crate::ecs::command_buffer::CommandBuffer;
use crate::ecs::system::System;
use crate::ecs::world::World;
use crate::game::ctf::nav::find_path;
use crate::game::ctf::resources::{
    AutoMovePath, AutoMoveState, CarrierState, ControlState, CtfInputState, CtfPointerState,
    CtfRestartState, CtfSyncState, EntityRefs, FlagMotion, FlagMotionState, GameState,
    NavigationGrid,
};
use crate::game::ctf::{
    ACTION_RESTART, ACTION_SELECT_SLOTS, ACTION_SWITCH, ACTION_TAG, FLAG_RADIUS,
    FLAG_THROW_DISTANCE, PLAYER_SPEED,
};
use crate::math::Vec3;
use crate::multiplayer::matchmaking::CtfSlot;
use crate::multiplayer::matchmaking::MapSize;
use crate::multiplayer::{CtfFlagId, CtfFlagThrowInput, InputFrame};

use super::is_playing;

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
        let (restart_requested, restart_key_changed) = update_restart_state(&frames, &mut controls);
        if let Some(requested_map_size) = restart_requested {
            if !is_playing(world) {
                restart_ctf_match(world, commands, &refs, controls, requested_map_size);
                return;
            }
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

        let mut carrier = world
            .resource::<CarrierState>()
            .copied()
            .unwrap_or_default();
        let original_carrier = carrier;
        let mut flag_motion = world
            .resource::<FlagMotionState>()
            .copied()
            .unwrap_or_default();
        let original_flag_motion = flag_motion;
        let mut auto_move = world
            .resource::<AutoMoveState>()
            .cloned()
            .unwrap_or_default();
        let original_auto_move = auto_move.clone();
        let nav_grid = world.resource::<NavigationGrid>().cloned();
        let midline_x = world
            .resource::<MapSize>()
            .map(|ms| ms.midline_x())
            .unwrap_or(crate::game::ctf::MIDLINE_X);

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
                midline_x,
            ) {
                dirty = true;
            }
        }

        // Reset non-selected slot velocities to zero so repulsion below cannot
        // accumulate across ticks (non-selected slots should be stationary).
        let selected: std::collections::HashSet<CtfSlot> = controls
            .controls
            .iter()
            .map(|c| c.selected_slot)
            .collect();
        for slot in CtfSlot::ALL {
            if !selected.contains(&slot) {
                velocities[slot.index()] = Velocity { dx: 0.0, dy: 0.0 };
            }
        }

        apply_flag_repulsion(
            &player_transforms,
            &mut velocities,
            &red_flag_tf,
            &blue_flag_tf,
            &carrier,
            &flag_motion,
        );

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
            commands.insert_resource(CtfSyncState::dirty());
        }
    }
}

fn update_restart_state(
    frames: &[InputFrame],
    controls: &mut ControlState,
) -> (Option<Option<MapSize>>, bool) {
    let mut restart_requested = None;
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
        if restart_down && !control.restart_down && restart_requested.is_none() {
            restart_requested = Some(frame.ctf_restart.map(|restart| restart.map_size));
        }
        changed |= restart_down != control.restart_down;
        control.restart_down = restart_down;
    }
    (restart_requested, changed)
}

fn restart_ctf_match(
    world: &World,
    commands: &mut CommandBuffer,
    refs: &EntityRefs,
    controls: ControlState,
    requested_map_size: Option<MapSize>,
) {
    let current_map_size = world
        .resource::<MapSize>()
        .copied()
        .unwrap_or(MapSize::Small);
    let fallback_map_size = world
        .resource::<CtfRestartState>()
        .map_or(current_map_size, |state| state.selected_map_size);
    let map_size = requested_map_size.unwrap_or(fallback_map_size);

    if map_size == current_map_size {
        reset_ctf_match(world, commands, refs, controls);
    } else {
        commands.insert_resource(CtfRestartState {
            selected_map_size: map_size,
            pending_reload: Some(map_size),
        });
        commands.insert_resource(CtfInputState::default());
        commands.insert_resource(controls);
        commands.insert_resource(CtfSyncState::dirty());
    }
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
        next.position = Vec3::new(
            refs.red_flag_spawn.0,
            refs.red_flag_spawn.1,
            transform.position.z,
        );
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
    commands.insert_resource(CtfSyncState::dirty());
}

fn apply_frame(
    frame: &InputFrame,
    _refs: &EntityRefs,
    controls: &mut ControlState,
    carrier: &mut CarrierState,
    player_transforms: &mut [Transform; CtfSlot::COUNT],
    velocities: &mut [Velocity; CtfSlot::COUNT],
    red_flag_tf: &mut Transform,
    blue_flag_tf: &mut Transform,
    flag_motion: &mut FlagMotionState,
    auto_move: &mut AutoMoveState,
    nav_grid: Option<&NavigationGrid>,
    _midline_x: f32,
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
    let throw_pressed = tag_down && !control.tag_down;
    let switch_pressed = switch_down && !control.switch_down;
    let mut dirty = false;
    let previous_slot = control.selected_slot;
    dirty |= control.normalize_selected_slot();

    if switch_pressed && control.owned_slots.len() > 1 {
        let current_index = control
            .owned_slots
            .iter()
            .position(|slot| *slot == control.selected_slot)
            .unwrap_or(0);
        control.selected_slot =
            control.owned_slots[(current_index + 1) % control.owned_slots.len()];
        dirty = true;
    }

    if let Some(slot) = requested_slot(control, frame.action_bits) {
        if control.selected_slot != slot {
            control.selected_slot = slot;
            dirty = true;
        }
    }

    let mut selected_slot = control.selected_slot;
    let has_keyboard = frame.move_x.abs() > f32::EPSILON || frame.move_y.abs() > f32::EPSILON;

    if has_keyboard {
        if auto_move.paths[selected_slot.index()].take().is_some() {
            dirty = true;
        }
    } else if let Some(path_start) = frame.ctf_auto_move {
        if control.owned_slots.contains(&path_start.slot) {
            selected_slot = path_start.slot;
            if control.selected_slot != selected_slot {
                control.selected_slot = selected_slot;
                dirty = true;
            }
            if set_auto_move_path(
                auto_move,
                selected_slot,
                path_start.start_world,
                path_start.target_world,
                nav_grid,
            ) {
                dirty = true;
            }
        }
    } else if let Some(click_world) = frame
        .ctf_pointer
        .as_ref()
        .and_then(|pointer| pointer.click_world)
    {
        let start = (
            player_transforms[selected_slot.index()].position.x,
            player_transforms[selected_slot.index()].position.y,
        );
        if set_auto_move_path(auto_move, selected_slot, start, click_world, nav_grid) {
            dirty = true;
        }
    }

    if selected_slot != previous_slot {
        velocities[previous_slot.index()] = Velocity { dx: 0.0, dy: 0.0 };
    }

    let acted_throw_event = apply_flag_throw_event(
        frame.ctf_flag_throw,
        control,
        carrier,
        red_flag_tf,
        blue_flag_tf,
        flag_motion,
    );
    if acted_throw_event {
        dirty = true;
    } else if throw_pressed {
        let acted = throw_carried_flag(
            selected_slot,
            frame
                .ctf_pointer
                .as_ref()
                .and_then(|pointer| pointer.aim_world),
            carrier,
            player_transforms,
            red_flag_tf,
            blue_flag_tf,
            flag_motion,
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

fn apply_flag_throw_event(
    event: Option<CtfFlagThrowInput>,
    control: &mut crate::game::ctf::resources::PlayerControl,
    carrier: &mut CarrierState,
    red_flag_tf: &mut Transform,
    blue_flag_tf: &mut Transform,
    flag_motion: &mut FlagMotionState,
) -> bool {
    let Some(event) = event else {
        return false;
    };
    if !control.owned_slots.contains(&event.slot) {
        return false;
    }

    control.selected_slot = event.slot;
    let start = Vec3::new(event.start_world.0, event.start_world.1, 0.0);
    let motion = Some(FlagMotion {
        dir_x: event.dir_x,
        dir_y: event.dir_y,
        remaining_distance: FLAG_THROW_DISTANCE,
        released_by: Some(event.slot),
    });

    match event.flag {
        CtfFlagId::Blue
            if carrier
                .blue_flag_carrier
                .map_or(true, |carrier| carrier == event.slot) =>
        {
            carrier.blue_flag_carrier = None;
            blue_flag_tf.position = Vec3::new(start.x, start.y, blue_flag_tf.position.z);
            flag_motion.blue = motion;
            true
        }
        CtfFlagId::Red
            if carrier
                .red_flag_carrier
                .map_or(true, |carrier| carrier == event.slot) =>
        {
            carrier.red_flag_carrier = None;
            red_flag_tf.position = Vec3::new(start.x, start.y, red_flag_tf.position.z);
            flag_motion.red = motion;
            true
        }
        _ => false,
    }
}

fn requested_slot(
    control: &crate::game::ctf::resources::PlayerControl,
    action_bits: u8,
) -> Option<CtfSlot> {
    ACTION_SELECT_SLOTS
        .iter()
        .enumerate()
        .filter_map(|(idx, action)| {
            if action_bits & action == 0 {
                return None;
            }
            CtfSlot::from_team_number(control.primary_slot.is_red(), idx as u8 + 1)
        })
        .find(|slot| control.owned_slots.contains(slot))
}

fn set_auto_move_path(
    auto_move: &mut AutoMoveState,
    slot: CtfSlot,
    start: (f32, f32),
    target: (f32, f32),
    nav_grid: Option<&NavigationGrid>,
) -> bool {
    let Some(grid) = nav_grid else {
        return false;
    };

    match find_path(grid, start, target) {
        Some(waypoints) if !waypoints.is_empty() => {
            auto_move.paths[slot.index()] = Some(AutoMovePath {
                waypoints,
                next_index: 0,
            });
            true
        }
        _ => auto_move.paths[slot.index()].take().is_some(),
    }
}

fn throw_carried_flag(
    slot: CtfSlot,
    aim_world: Option<(f32, f32)>,
    carrier: &mut CarrierState,
    player_transforms: &[Transform; CtfSlot::COUNT],
    red_flag_tf: &mut Transform,
    blue_flag_tf: &mut Transform,
    flag_motion: &mut FlagMotionState,
) -> bool {
    let throws_blue = carrier.blue_flag_carrier == Some(slot);
    let throws_red = carrier.red_flag_carrier == Some(slot);
    if !throws_blue && !throws_red {
        return false;
    }

    let player_pos = player_transforms[slot.index()].position;
    let (dir_x, dir_y) = throw_direction(slot, player_pos.x, player_pos.y, aim_world);
    let motion = Some(FlagMotion {
        dir_x,
        dir_y,
        remaining_distance: FLAG_THROW_DISTANCE,
        released_by: Some(slot),
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

fn apply_flag_repulsion(
    player_transforms: &[Transform; CtfSlot::COUNT],
    velocities: &mut [Velocity; CtfSlot::COUNT],
    red_flag_tf: &Transform,
    blue_flag_tf: &Transform,
    carrier: &CarrierState,
    flag_motion: &FlagMotionState,
) {
    let protection_radius_sq = (5.0 * FLAG_RADIUS) * (5.0 * FLAG_RADIUS);
    let repulsion_speed = 2.0 * PLAYER_SPEED;

    if carrier.red_flag_carrier.is_none() && flag_motion.red.is_none() {
        for slot in CtfSlot::RED {
            repel_from_flag(
                slot,
                player_transforms,
                velocities,
                red_flag_tf,
                protection_radius_sq,
                repulsion_speed,
            );
        }
    }

    if carrier.blue_flag_carrier.is_none() && flag_motion.blue.is_none() {
        for slot in CtfSlot::BLUE {
            repel_from_flag(
                slot,
                player_transforms,
                velocities,
                blue_flag_tf,
                protection_radius_sq,
                repulsion_speed,
            );
        }
    }
}

fn repel_from_flag(
    slot: CtfSlot,
    player_transforms: &[Transform; CtfSlot::COUNT],
    velocities: &mut [Velocity; CtfSlot::COUNT],
    flag_tf: &Transform,
    protection_radius_sq: f32,
    repulsion_speed: f32,
) {
    let player_pos = &player_transforms[slot.index()].position;
    let dx = player_pos.x - flag_tf.position.x;
    let dy = player_pos.y - flag_tf.position.y;
    let dist_sq = dx * dx + dy * dy;
    if dist_sq >= protection_radius_sq || dist_sq <= f32::EPSILON {
        return;
    }
    let dist = dist_sq.sqrt();
    velocities[slot.index()].dx += dx / dist * repulsion_speed;
    velocities[slot.index()].dy += dy / dist * repulsion_speed;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::command_buffer::CommandBuffer;
    use crate::game::ctf::resources::{
        AutoMovePath, AutoMoveState, CtfRestartState, FlagMotion, FlagMotionState, GamePhase,
        GameState,
    };
    use crate::game::ctf::setup::setup_ctf_scene_entities;
    use crate::game::ctf::systems::TaggingSystem;
    use crate::multiplayer::matchmaking::CtfSlotAssignment;
    use crate::multiplayer::{
        CtfAutoMoveStartInput, CtfFlagId, CtfFlagThrowInput, CtfPointerInput, CtfRestartInput,
    };
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
                ctf_restart: None,
                ctf_auto_move: None,
                ctf_flag_throw: None,
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
    fn number_keys_select_only_owned_team_slots() {
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
                    primary_slot: CtfSlot::Red3,
                },
            ],
            MapSize::Small,
        );
        world.insert_resource(CtfInputState {
            frames: vec![
                InputFrame {
                    tick: 1,
                    player_id: 1,
                    move_x: 0.0,
                    move_y: 0.0,
                    action_bits: ACTION_SELECT_SLOTS[2],
                    ctf_pointer: None,
                    ctf_restart: None,
                    ctf_auto_move: None,
                    ctf_flag_throw: None,
                },
                InputFrame {
                    tick: 1,
                    player_id: 2,
                    move_x: 0.0,
                    move_y: 0.0,
                    action_bits: ACTION_SELECT_SLOTS[3],
                    ctf_pointer: None,
                    ctf_restart: None,
                    ctf_auto_move: None,
                    ctf_flag_throw: None,
                },
            ],
        });

        let mut commands = CommandBuffer::new();
        CtfInputSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        let controls = world.resource::<ControlState>().expect("controls");
        let red_a = controls
            .controls
            .iter()
            .find(|control| control.client_id == 1)
            .expect("red A control");
        let red_b = controls
            .controls
            .iter()
            .find(|control| control.client_id == 2)
            .expect("red B control");
        assert_eq!(red_a.selected_slot, CtfSlot::Red1);
        assert_eq!(red_b.selected_slot, CtfSlot::Red4);
    }

    #[test]
    fn switching_control_zeroes_previous_slot_velocity() {
        let mut world = test_world();
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
        world.insert(
            refs.player(CtfSlot::Red1),
            Velocity {
                dx: PLAYER_SPEED,
                dy: 0.0,
            },
        );
        world.insert_resource(CtfInputState {
            frames: vec![InputFrame {
                tick: 1,
                player_id: 1,
                move_x: 1.0,
                move_y: 0.0,
                action_bits: ACTION_SELECT_SLOTS[1],
                ctf_pointer: None,
                ctf_restart: None,
                ctf_auto_move: None,
                ctf_flag_throw: None,
            }],
        });

        let mut commands = CommandBuffer::new();
        CtfInputSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        let red1_velocity = world
            .get::<Velocity>(refs.player(CtfSlot::Red1))
            .expect("red1 velocity");
        let red2_velocity = world
            .get::<Velocity>(refs.player(CtfSlot::Red2))
            .expect("red2 velocity");
        assert_eq!((red1_velocity.dx, red1_velocity.dy), (0.0, 0.0));
        assert_eq!((red2_velocity.dx, red2_velocity.dy), (PLAYER_SPEED, 0.0));
    }

    #[test]
    fn input_repairs_invalid_selected_slot() {
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
                    primary_slot: CtfSlot::Red3,
                },
            ],
            MapSize::Small,
        );
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
        if let Some(mut controls) = world.resource::<ControlState>().cloned() {
            controls
                .controls
                .iter_mut()
                .find(|control| control.client_id == 1)
                .expect("red control")
                .selected_slot = CtfSlot::Red3;
            world.insert_resource(controls);
        }
        world.insert(
            refs.player(CtfSlot::Red3),
            Velocity {
                dx: PLAYER_SPEED,
                dy: 0.0,
            },
        );
        world.insert_resource(CtfInputState {
            frames: vec![InputFrame {
                tick: 1,
                player_id: 1,
                move_x: 1.0,
                move_y: 0.0,
                action_bits: 0,
                ctf_pointer: None,
                ctf_restart: None,
                ctf_auto_move: None,
                ctf_flag_throw: None,
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
        let red1_velocity = world
            .get::<Velocity>(refs.player(CtfSlot::Red1))
            .expect("red1 velocity");
        let red3_velocity = world
            .get::<Velocity>(refs.player(CtfSlot::Red3))
            .expect("red3 velocity");
        assert_eq!(red.selected_slot, CtfSlot::Red1);
        assert_eq!((red1_velocity.dx, red1_velocity.dy), (PLAYER_SPEED, 0.0));
        assert_eq!((red3_velocity.dx, red3_velocity.dy), (0.0, 0.0));
    }

    #[test]
    fn tag_teleports_non_carrier_opponent_home() {
        let mut world = test_world();
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
        let blue = refs.player(CtfSlot::Blue1);
        let mut blue_tf = world.get::<Transform>(blue).cloned().expect("blue tf");
        blue_tf.position.x = 320.0;
        blue_tf.position.y = 300.0;
        world.insert(blue, blue_tf);

        run_tagging(&mut world);

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
        blue_tf.position.x = 320.0;
        blue_tf.position.y = 300.0;
        world.insert(blue, blue_tf);
        world.insert_resource(CarrierState {
            red_flag_carrier: Some(CtfSlot::Blue1),
            blue_flag_carrier: None,
        });

        run_tagging(&mut world);

        let carrier = world.resource::<CarrierState>().expect("carrier");
        assert_eq!(carrier.red_flag_carrier, None);
        let red_flag = world
            .get::<Transform>(refs.red_flag)
            .expect("red flag transform");
        assert_eq!((red_flag.position.x, red_flag.position.y), (320.0, 300.0));
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
                    aim_world: Some((360.0, 300.0)),
                    click_world: None,
                }),
                ctf_restart: None,
                ctf_auto_move: None,
                ctf_flag_throw: None,
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
                released_by: Some(CtfSlot::Red1),
            })
        );
    }

    #[test]
    fn carrier_action_uses_announced_flag_throw() {
        let mut world = test_world();
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
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
                    aim_world: Some((260.0, 300.0)),
                    click_world: None,
                }),
                ctf_restart: None,
                ctf_auto_move: None,
                ctf_flag_throw: Some(CtfFlagThrowInput {
                    slot: CtfSlot::Red1,
                    flag: CtfFlagId::Blue,
                    start_world: (200.0, 333.0),
                    dir_x: 0.0,
                    dir_y: 1.0,
                }),
            }],
        });

        let mut commands = CommandBuffer::new();
        CtfInputSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        let carrier = world.resource::<CarrierState>().expect("carrier");
        assert_eq!(carrier.blue_flag_carrier, None);
        let blue_flag = world
            .get::<Transform>(refs.blue_flag)
            .expect("blue flag transform");
        assert_eq!((blue_flag.position.x, blue_flag.position.y), (200.0, 333.0));
        let motion = world.resource::<FlagMotionState>().expect("flag motion");
        assert_eq!(
            motion.blue,
            Some(FlagMotion {
                dir_x: 0.0,
                dir_y: 1.0,
                remaining_distance: crate::game::ctf::FLAG_THROW_DISTANCE,
                released_by: Some(CtfSlot::Red1),
            })
        );
    }

    #[test]
    fn announced_own_flag_throw_tolerates_lagging_empty_carrier_state() {
        let mut world = test_world();
        world.insert_resource(CarrierState::default());
        world.insert_resource(CtfInputState {
            frames: vec![InputFrame {
                tick: 1,
                player_id: 1,
                move_x: 0.0,
                move_y: 0.0,
                action_bits: ACTION_TAG,
                ctf_pointer: None,
                ctf_restart: None,
                ctf_auto_move: None,
                ctf_flag_throw: Some(CtfFlagThrowInput {
                    slot: CtfSlot::Red1,
                    flag: CtfFlagId::Red,
                    start_world: (260.0, 300.0),
                    dir_x: 1.0,
                    dir_y: 0.0,
                }),
            }],
        });

        let mut commands = CommandBuffer::new();
        CtfInputSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        let carrier = world.resource::<CarrierState>().expect("carrier");
        assert_eq!(carrier.red_flag_carrier, None);
        let motion = world.resource::<FlagMotionState>().expect("flag motion");
        assert_eq!(
            motion.red,
            Some(FlagMotion {
                dir_x: 1.0,
                dir_y: 0.0,
                remaining_distance: crate::game::ctf::FLAG_THROW_DISTANCE,
                released_by: Some(CtfSlot::Red1),
            })
        );
    }

    #[test]
    fn announced_flag_throw_ignores_stale_tag_down_edge_state() {
        let mut world = test_world();
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
        world.insert_resource(CarrierState {
            red_flag_carrier: None,
            blue_flag_carrier: Some(CtfSlot::Red1),
        });
        if let Some(mut controls) = world.resource::<ControlState>().cloned() {
            controls
                .controls
                .iter_mut()
                .find(|control| control.client_id == 1)
                .expect("red control")
                .tag_down = true;
            world.insert_resource(controls);
        }
        world.insert_resource(CtfInputState {
            frames: vec![InputFrame {
                tick: 1,
                player_id: 1,
                move_x: 0.0,
                move_y: 0.0,
                action_bits: ACTION_TAG,
                ctf_pointer: None,
                ctf_restart: None,
                ctf_auto_move: None,
                ctf_flag_throw: Some(CtfFlagThrowInput {
                    slot: CtfSlot::Red1,
                    flag: CtfFlagId::Blue,
                    start_world: (200.0, 333.0),
                    dir_x: 0.0,
                    dir_y: 1.0,
                }),
            }],
        });

        let mut commands = CommandBuffer::new();
        CtfInputSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        let carrier = world.resource::<CarrierState>().expect("carrier");
        assert_eq!(carrier.blue_flag_carrier, None);
        let blue_flag = world
            .get::<Transform>(refs.blue_flag)
            .expect("blue flag transform");
        assert_eq!((blue_flag.position.x, blue_flag.position.y), (200.0, 333.0));
        let motion = world.resource::<FlagMotionState>().expect("flag motion");
        assert_eq!(
            motion.blue,
            Some(FlagMotion {
                dir_x: 0.0,
                dir_y: 1.0,
                remaining_distance: crate::game::ctf::FLAG_THROW_DISTANCE,
                released_by: Some(CtfSlot::Red1),
            })
        );
    }

    #[test]
    fn announced_flag_throw_does_not_override_conflicting_carrier() {
        let mut world = test_world();
        world.insert_resource(CarrierState {
            red_flag_carrier: Some(CtfSlot::Red2),
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
                ctf_restart: None,
                ctf_auto_move: None,
                ctf_flag_throw: Some(CtfFlagThrowInput {
                    slot: CtfSlot::Red1,
                    flag: CtfFlagId::Red,
                    start_world: (260.0, 300.0),
                    dir_x: 1.0,
                    dir_y: 0.0,
                }),
            }],
        });

        let mut commands = CommandBuffer::new();
        CtfInputSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        let carrier = world.resource::<CarrierState>().expect("carrier");
        assert_eq!(carrier.red_flag_carrier, Some(CtfSlot::Red2));
        let motion = world.resource::<FlagMotionState>().expect("flag motion");
        assert_eq!(motion.red, None);
    }

    #[test]
    fn carrier_action_throws_own_flag() {
        let mut world = test_world();
        world.insert_resource(CarrierState {
            red_flag_carrier: Some(CtfSlot::Red1),
            blue_flag_carrier: None,
        });
        world.insert_resource(CtfInputState {
            frames: vec![InputFrame {
                tick: 1,
                player_id: 1,
                move_x: 0.0,
                move_y: 0.0,
                action_bits: ACTION_TAG,
                ctf_pointer: Some(CtfPointerInput {
                    aim_world: Some((360.0, 300.0)),
                    click_world: None,
                }),
                ctf_restart: None,
                ctf_auto_move: None,
                ctf_flag_throw: None,
            }],
        });

        let mut commands = CommandBuffer::new();
        CtfInputSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        let carrier = world.resource::<CarrierState>().expect("carrier");
        assert_eq!(carrier.red_flag_carrier, None);
        let motion = world.resource::<FlagMotionState>().expect("flag motion");
        assert_eq!(
            motion.red,
            Some(FlagMotion {
                dir_x: 1.0,
                dir_y: 0.0,
                remaining_distance: crate::game::ctf::FLAG_THROW_DISTANCE,
                released_by: Some(CtfSlot::Red1),
            })
        );
    }

    #[test]
    fn owner_action_knocks_down_nearby_moving_flag() {
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
            red: Some(FlagMotion {
                dir_x: 1.0,
                dir_y: 0.0,
                remaining_distance: 80.0,
                released_by: None,
            }),
            blue: None,
        });
        run_tagging(&mut world);

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
                ctf_restart: None,
                ctf_auto_move: None,
                ctf_flag_throw: None,
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
    fn auto_move_start_sets_path_from_announced_slot() {
        let mut world = test_world();
        world.insert_resource(CtfInputState {
            frames: vec![InputFrame {
                tick: 1,
                player_id: 1,
                move_x: 0.0,
                move_y: 0.0,
                action_bits: 0,
                ctf_pointer: None,
                ctf_restart: None,
                ctf_auto_move: Some(CtfAutoMoveStartInput {
                    slot: CtfSlot::Red1,
                    start_world: (180.0, 320.0),
                    target_world: (260.0, 320.0),
                }),
                ctf_flag_throw: None,
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
    fn missing_remote_frame_keeps_previous_velocity_for_extrapolation() {
        let mut world = test_world();
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
        world.insert(refs.player(CtfSlot::Blue1), Velocity { dx: 12.0, dy: -3.0 });
        world.insert_resource(CtfInputState {
            frames: vec![InputFrame {
                tick: 1,
                player_id: 1,
                move_x: 1.0,
                move_y: 0.0,
                action_bits: 0,
                ctf_pointer: None,
                ctf_restart: None,
                ctf_auto_move: None,
                ctf_flag_throw: None,
            }],
        });

        let mut commands = CommandBuffer::new();
        CtfInputSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        let blue_velocity = world
            .get::<Velocity>(refs.player(CtfSlot::Blue1))
            .expect("blue velocity");
        assert_eq!((blue_velocity.dx, blue_velocity.dy), (12.0, -3.0));
    }

    #[test]
    fn restart_action_is_ignored_while_playing() {
        let mut world = test_world();
        let refs = world.resource::<EntityRefs>().cloned().expect("refs");
        let red = refs.player(CtfSlot::Red1);
        let mut red_tf = world.get::<Transform>(red).cloned().expect("red tf");
        red_tf.position.x = 500.0;
        red_tf.position.y = 500.0;
        world.insert(red, red_tf);
        world.insert_resource(CtfInputState {
            frames: vec![InputFrame {
                tick: 1,
                player_id: 1,
                move_x: 0.0,
                move_y: 0.0,
                action_bits: ACTION_RESTART,
                ctf_pointer: None,
                ctf_restart: None,
                ctf_auto_move: None,
                ctf_flag_throw: None,
            }],
        });

        let mut commands = CommandBuffer::new();
        CtfInputSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        assert!(matches!(
            world.resource::<GameState>().expect("game state").phase,
            GamePhase::Playing
        ));
        let red_tf = world.get::<Transform>(red).expect("red tf");
        assert_eq!((red_tf.position.x, red_tf.position.y), (500.0, 500.0));
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
        world.insert(
            red,
            Velocity {
                dx: 123.0,
                dy: 45.0,
            },
        );
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
                released_by: None,
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
                ctf_restart: None,
                ctf_auto_move: None,
                ctf_flag_throw: None,
            }],
        });

        let mut commands = CommandBuffer::new();
        CtfInputSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        assert!(matches!(
            world.resource::<GameState>().expect("game state").phase,
            GamePhase::Playing
        ));
        assert_eq!(
            world.resource::<CarrierState>().expect("carrier"),
            &CarrierState::default()
        );
        assert_eq!(
            *world.resource::<FlagMotionState>().expect("flag motion"),
            FlagMotionState::default()
        );
        assert_eq!(
            *world.resource::<AutoMoveState>().expect("auto move"),
            AutoMoveState::default()
        );
        let red_tf = world.get::<Transform>(red).expect("red tf");
        assert_eq!(
            (red_tf.position.x, red_tf.position.y),
            refs.player_spawn(CtfSlot::Red1)
        );
        let red_vel = world.get::<Velocity>(red).expect("red velocity");
        assert_eq!((red_vel.dx, red_vel.dy), (0.0, 0.0));
        let blue_flag_tf = world.get::<Transform>(refs.blue_flag).expect("blue flag");
        assert_eq!(
            (blue_flag_tf.position.x, blue_flag_tf.position.y),
            refs.blue_flag_spawn
        );
    }

    #[test]
    fn restart_action_requests_selected_level_reload_after_win() {
        let mut world = test_world();
        world.insert_resource(GameState {
            phase: GamePhase::Won(1),
        });
        world.insert_resource(CtfRestartState {
            selected_map_size: MapSize::Medium,
            pending_reload: None,
        });
        world.insert_resource(CtfInputState {
            frames: vec![InputFrame {
                tick: 1,
                player_id: 1,
                move_x: 0.0,
                move_y: 0.0,
                action_bits: ACTION_RESTART,
                ctf_pointer: None,
                ctf_restart: Some(CtfRestartInput {
                    map_size: MapSize::Medium,
                }),
                ctf_auto_move: None,
                ctf_flag_throw: None,
            }],
        });

        let mut commands = CommandBuffer::new();
        CtfInputSystem.run(&world, &mut commands);
        commands.flush(&mut world);

        let restart = world.resource::<CtfRestartState>().expect("restart");
        assert_eq!(restart.selected_map_size, MapSize::Medium);
        assert_eq!(restart.pending_reload, Some(MapSize::Medium));
        assert!(matches!(
            world.resource::<GameState>().expect("game state").phase,
            GamePhase::Won(1)
        ));
    }
}
