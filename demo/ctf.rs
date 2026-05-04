//! Local debug runner for the scene-backed capture-the-flag game.
//!
//! Run with: `cargo run --bin ctf`

#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::too_many_lines)]

use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use forge_ecs::app::AppCore;
use forge_ecs::components::{Color, Shape, Transform};
use forge_ecs::ecs::resource::{DeltaTime, ElapsedTime, KeysPressed};
use forge_ecs::ecs::world::World;
use forge_ecs::game::ctf::resources::{
    CtfInputState, CtfPointerState, CtfRestartState, EntityRefs,
};
use forge_ecs::game::ctf::setup::setup_ctf_scene_entities;
use forge_ecs::game::ctf::systems::hud::{draw_hud, draw_player_numbers};
use forge_ecs::game::ctf::systems::{
    AutoMoveSystem, CtfInputSystem, FlagCarrySystem, FlagMotionSystem, FlagPickupSystem,
    StopOnWinSystem, TaggingSystem, WallCollisionSystem, WinConditionSystem,
};
use forge_ecs::game::ctf::{
    ACTION_RESTART, ACTION_SELECT_SLOTS, ACTION_SWITCH, ACTION_TAG, ARENA_HEIGHT, ARENA_WIDTH,
    RESTART_KEY, SLOT_SELECT_KEYS, SWITCH_KEY,
};
use forge_ecs::messaging::{LoopPhase, MessageBus};
use forge_ecs::multiplayer::matchmaking::{CtfSlot, CtfSlotAssignment, MapSize};
use forge_ecs::multiplayer::{CtfPointerInput, CtfRestartInput, InputFrame};
use forge_ecs::platform::{map_window_event, KeyCode, MouseButton, PlatformEvent};
use forge_ecs::renderer::draw::DrawCommand;
use forge_ecs::scene::reload_scene;
use forge_ecs::systems::MovementSystem;

fn main() {
    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut app = CtfApp { state: None };
    event_loop.run_app(&mut app).expect("event loop error");
}

struct CtfApp {
    state: Option<CtfState>,
}

struct CtfState {
    core: AppCore,
    bus: MessageBus,
    last_time: Instant,
    cursor_screen: Option<(f64, f64)>,
    /// Index into `MapSize::ALL`; drives the pre-game selector.
    selected_map_idx: usize,
    /// False until the player confirms a map choice and the world is loaded.
    game_started: bool,
}

impl ApplicationHandler for CtfApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }

        let attrs = WindowAttributes::default()
            .with_title("Forge ECS -- Capture the Flag")
            .with_inner_size(PhysicalSize::new(ARENA_WIDTH as u32, ARENA_HEIGHT as u32))
            .with_resizable(false);
        let window: Window = event_loop
            .create_window(attrs)
            .expect("window creation failed");
        let core = AppCore::from_window(window).expect("AppCore creation failed");

        // World setup is deferred until the player selects a map size.
        self.state = Some(CtfState {
            core,
            bus: build_bus(),
            last_time: Instant::now(),
            cursor_screen: None,
            selected_map_idx: 0,
            game_started: false,
        });
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        if let Some(state) = &mut self.state {
            let full = winit::event::Event::<()>::NewEvents(cause);
            state
                .core
                .imgui
                .handle_event(state.core.platform.window(), &full);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = &mut self.state else {
            return;
        };
        if window_id != state.core.platform.window.id() {
            return;
        }

        state
            .core
            .imgui
            .handle_window_event(state.core.platform.window(), window_id, &event);

        match &event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                state.core.render_ctx.resize(size.width, size.height);
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let size = state.core.platform.window.inner_size();
                state.core.render_ctx.resize(size.width, size.height);
            }
            WindowEvent::Focused(false) => {
                if let Some(keys) = state.core.world.resource_mut::<KeysPressed>() {
                    keys.clear();
                }
            }
            WindowEvent::RedrawRequested => render(state),
            _ => {}
        }

        if let Some(platform_event) = map_window_event(&event) {
            match platform_event {
                PlatformEvent::KeyPressed(code) => {
                    if let Some(discriminant) = key_discriminant(code) {
                        if let Some(keys) = state.core.world.resource_mut::<KeysPressed>() {
                            keys.press(discriminant);
                        }
                    }
                }
                PlatformEvent::KeyReleased(code) => {
                    if let Some(discriminant) = key_discriminant(code) {
                        if let Some(keys) = state.core.world.resource_mut::<KeysPressed>() {
                            keys.release(discriminant);
                        }
                    }
                }
                PlatformEvent::MouseMoved { x, y } => {
                    state.cursor_screen = Some((x, y));
                    if let Some(pointer) = state.core.world.resource_mut::<CtfPointerState>() {
                        pointer.cursor_world = Some((x as f32, y as f32));
                    }
                }
                PlatformEvent::MouseButton {
                    button: MouseButton::Left,
                    pressed: true,
                } => {
                    if let Some((x, y)) = state.cursor_screen {
                        if let Some(pointer) = state.core.world.resource_mut::<CtfPointerState>() {
                            let world_pos = (x as f32, y as f32);
                            pointer.cursor_world = Some(world_pos);
                            pointer.pending_left_click_world = Some(world_pos);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        let Some(state) = &mut self.state else {
            return;
        };

        state
            .core
            .imgui
            .handle_about_to_wait(state.core.platform.window());

        let now = Instant::now();
        let dt = now.duration_since(state.last_time).as_secs_f32();
        state.last_time = now;

        if state.game_started {
            if let Some(resource) = state.core.world.resource_mut::<DeltaTime>() {
                resource.0 = dt;
            }
            if let Some(resource) = state.core.world.resource_mut::<ElapsedTime>() {
                resource.0 += dt;
            }
            queue_local_debug_inputs(&mut state.core.world);
            state.bus.run_frame(&mut state.core.world);
            process_ctf_level_reload(state);
        }

        state.core.platform.window.request_redraw();
    }
}

fn build_bus() -> MessageBus {
    let mut bus = MessageBus::new();
    bus.register(LoopPhase::Update, -10, CtfInputSystem);
    bus.register(LoopPhase::Update, -8, AutoMoveSystem);
    bus.register(LoopPhase::Update, -7, StopOnWinSystem);
    bus.register(LoopPhase::Update, MovementSystem::PRIORITY, MovementSystem);
    bus.register(LoopPhase::Update, 2, WallCollisionSystem);
    bus.register(LoopPhase::Update, 4, FlagMotionSystem);
    bus.register(LoopPhase::Update, 5, TaggingSystem);
    bus.register(LoopPhase::Update, 6, FlagPickupSystem);
    bus.register(LoopPhase::Update, 10, FlagCarrySystem);
    bus.register(LoopPhase::Update, 20, WinConditionSystem);
    bus
}

fn setup_ctf_world(world: &mut World, map: MapSize) {
    let path = map.scene_path();
    reload_scene(world, path).unwrap_or_else(|err| {
        panic!("failed to load {path}: {err}");
    });

    world.insert_resource(KeysPressed::default());
    setup_ctf_scene_entities(
        world,
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
        map,
    );
}

fn queue_local_debug_inputs(world: &mut World) {
    let keys = world.resource::<KeysPressed>().cloned().unwrap_or_default();
    let pointer_input = take_ctf_pointer_input(world);
    let mut frames = Vec::new();
    let red_move_x = axis(&keys, 5, 7);
    let red_move_y = axis(&keys, 4, 6);
    let blue_move_x = axis(&keys, 0, 1);
    let blue_move_y = axis(&keys, 2, 3);
    let (red_tag, blue_tag, switch, restart, slot_select) = world
        .resource_mut::<KeysPressed>()
        .map_or((false, false, false, false, 0), |keys| {
            (
                keys.consume_pressed(8),
                keys.consume_pressed(9),
                keys.consume_pressed(SWITCH_KEY),
                keys.consume_pressed(RESTART_KEY),
                slot_select_action_bits(keys),
            )
        });

    frames.push(InputFrame {
        tick: 0,
        player_id: 1,
        move_x: red_move_x,
        move_y: red_move_y,
        action_bits: (if red_tag { ACTION_TAG } else { 0 })
            | (if switch { ACTION_SWITCH } else { 0 })
            | (if restart { ACTION_RESTART } else { 0 })
            | slot_select,
        ctf_pointer: pointer_input,
        ctf_restart: ctf_restart_input(world, restart),
        ctf_auto_move: None,
        ctf_flag_throw: None,
    });
    frames.push(InputFrame {
        tick: 0,
        player_id: 2,
        move_x: blue_move_x,
        move_y: blue_move_y,
        action_bits: (if blue_tag { ACTION_TAG } else { 0 })
            | (if switch { ACTION_SWITCH } else { 0 })
            | (if restart { ACTION_RESTART } else { 0 })
            | slot_select,
        ctf_pointer: pointer_input.map(|pointer| CtfPointerInput {
            aim_world: pointer.aim_world,
            click_world: None,
        }),
        ctf_restart: ctf_restart_input(world, restart),
        ctf_auto_move: None,
        ctf_flag_throw: None,
    });
    world.insert_resource(CtfInputState { frames });
}

fn slot_select_action_bits(keys: &mut KeysPressed) -> u8 {
    SLOT_SELECT_KEYS
        .iter()
        .zip(ACTION_SELECT_SLOTS)
        .fold(0, |bits, (key, action)| {
            bits | if keys.consume_pressed(*key) {
                action
            } else {
                0
            }
        })
}

fn ctf_restart_input(world: &World, restart: bool) -> Option<CtfRestartInput> {
    if !restart {
        return None;
    }
    let map_size = world
        .resource::<CtfRestartState>()
        .map_or(MapSize::Small, |state| state.selected_map_size);
    Some(CtfRestartInput { map_size })
}

fn take_ctf_pointer_input(world: &mut World) -> Option<CtfPointerInput> {
    let pointer = world.resource_mut::<CtfPointerState>()?;
    Some(CtfPointerInput {
        aim_world: pointer.cursor_world,
        click_world: pointer.pending_left_click_world.take(),
    })
}

fn axis(keys: &KeysPressed, negative: u32, positive: u32) -> f32 {
    let raw = (if keys.is_held(positive) { 1.0_f32 } else { 0.0 })
        - (if keys.is_held(negative) { 1.0_f32 } else { 0.0 });
    raw
}

fn key_discriminant(code: KeyCode) -> Option<u32> {
    forge_ecs::systems::player_input::key_code_discriminant(code)
}

fn render(state: &mut CtfState) {
    state
        .core
        .render_ctx
        .sync_with_window(state.core.platform.window());

    if state.game_started {
        state
            .core
            .world
            .query3::<Transform, Shape, Color>()
            .for_each(|(_, transform, shape, color)| {
                state
                    .core
                    .draw_queue
                    .push(make_draw_cmd(transform, shape, color));
            });
    }

    let Some((surface_texture, view)) = state.core.render_ctx.begin_frame() else {
        return;
    };
    let mut encoder =
        state
            .core
            .render_ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ctf frame"),
            });

    state.core.draw_queue.flush(
        &state.core.render_ctx,
        &view,
        &mut encoder,
        &state.core.circle_pipeline,
        &state.core.rect_pipeline,
        &state.core.triangle_pipeline,
        [0.08, 0.09, 0.10, 1.0],
    );

    // Use a flag to defer world loading until after the imgui borrow ends.
    let mut start_game = false;
    {
        let ui = state.core.imgui.begin_frame(state.core.platform.window());
        if state.game_started {
            draw_hud(ui, &mut state.core.world);
            let positions = ctf_player_label_positions(&state.core.world);
            draw_player_numbers(ui, &positions);
        } else {
            draw_map_selection(ui, &mut state.selected_map_idx, &mut start_game);
        }
        state.core.imgui.end_frame(
            state.core.platform.window(),
            &state.core.render_ctx.device,
            &state.core.render_ctx.queue,
            &mut encoder,
            &view,
        );
    }
    if start_game {
        let map = MapSize::ALL[state.selected_map_idx];
        setup_ctf_world(&mut state.core.world, map);
        state.game_started = true;
    }

    state
        .core
        .render_ctx
        .queue
        .submit(std::iter::once(encoder.finish()));
    state.core.render_ctx.end_frame(surface_texture);
}

fn draw_map_selection(ui: &imgui::Ui, selected_idx: &mut usize, start_game: &mut bool) {
    let [w, h] = ui.io().display_size;
    let win_w = 300.0_f32;
    let win_h = 120.0_f32;
    ui.window("Select Map")
        .size([win_w, win_h], imgui::Condition::Always)
        .position(
            [(w - win_w) * 0.5, (h - win_h) * 0.5],
            imgui::Condition::Always,
        )
        .resizable(false)
        .collapsible(false)
        .build(|| {
            let labels = MapSize::ALL.map(MapSize::label);
            ui.combo_simple_string("Map Size", selected_idx, &labels);

            ui.separator();
            if ui.button("Play") {
                *start_game = true;
            }
        });
}

fn ctf_player_label_positions(world: &World) -> Vec<(CtfSlot, (f32, f32))> {
    let Some(refs) = world.resource::<EntityRefs>() else {
        return Vec::new();
    };
    CtfSlot::ALL
        .iter()
        .filter_map(|slot| {
            let transform = world.get::<Transform>(refs.player(*slot))?;
            Some((*slot, (transform.position.x, transform.position.y)))
        })
        .collect()
}

fn make_draw_cmd(transform: &Transform, shape: &Shape, color: &Color) -> DrawCommand {
    match shape {
        Shape::Circle { radius } => DrawCommand::Circle {
            x: transform.position.x,
            y: transform.position.y,
            radius: *radius,
            color: [color.r, color.g, color.b, color.a],
        },
        Shape::Rect { width, height } => DrawCommand::Rect {
            x: transform.position.x,
            y: transform.position.y,
            width: *width,
            height: *height,
            color: [color.r, color.g, color.b, color.a],
        },
        Shape::Triangle { size } => DrawCommand::Triangle {
            x: transform.position.x,
            y: transform.position.y,
            size: *size,
            rotation: transform.rotation,
            color: [color.r, color.g, color.b, color.a],
        },
    }
}

fn process_ctf_level_reload(state: &mut CtfState) {
    let Some(map_size) = state
        .core
        .world
        .resource::<CtfRestartState>()
        .and_then(|restart| restart.pending_reload)
    else {
        return;
    };

    setup_ctf_world(&mut state.core.world, map_size);
    state.selected_map_idx = MapSize::ALL
        .iter()
        .position(|candidate| *candidate == map_size)
        .unwrap_or(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_ecs::game::ctf::components::{Flag, PlayerMarker, Wall};
    use forge_ecs::game::ctf::resources::EntityRefs;

    #[test]
    fn setup_resolves_ctf_scene_entities() {
        let mut world = World::new();
        setup_ctf_world(&mut world, MapSize::Small);

        assert!(world.resource::<EntityRefs>().is_some());
        assert_eq!(world.query::<PlayerMarker>().count(), 8);
        assert_eq!(world.query::<Flag>().count(), 2);
        assert_eq!(world.query::<Wall>().count(), 16);
    }
}
