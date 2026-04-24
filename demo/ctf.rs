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
use forge_ecs::game::ctf::resources::{CtfInputState, CtfPointerState};
use forge_ecs::game::ctf::setup::setup_ctf_scene_entities;
use forge_ecs::game::ctf::systems::hud::draw_hud;
use forge_ecs::game::ctf::systems::{
    AutoMoveSystem, CtfInputSystem, FlagCarrySystem, FlagMotionSystem, FlagPickupSystem,
    StopOnWinSystem, WallCollisionSystem, WinConditionSystem,
};
use forge_ecs::game::ctf::{
    ACTION_RESTART, ACTION_SWITCH, ACTION_TAG, ARENA_HEIGHT, ARENA_WIDTH, RESTART_KEY,
    SWITCH_KEY,
};
use forge_ecs::messaging::{LoopPhase, MessageBus};
use forge_ecs::multiplayer::matchmaking::{CtfSlot, CtfSlotAssignment};
use forge_ecs::multiplayer::{CtfPointerInput, InputFrame};
use forge_ecs::platform::{map_window_event, KeyCode, MouseButton, PlatformEvent};
use forge_ecs::renderer::draw::DrawCommand;
use forge_ecs::scene::reload_scene;
use forge_ecs::systems::MovementSystem;

const SCENE_PATH: &str = "assets/ctf_scene.json";

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
        let mut core = AppCore::from_window(window).expect("AppCore creation failed");

        setup_ctf_world(&mut core.world);

        self.state = Some(CtfState {
            core,
            bus: build_bus(),
            last_time: Instant::now(),
            cursor_screen: None,
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

        state.core.imgui.handle_window_event(
            state.core.platform.window(),
            window_id,
            &event,
        );

        match &event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                state.core.render_ctx.resize(size.width, size.height);
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let size = state.core.platform.window.inner_size();
                state.core.render_ctx.resize(size.width, size.height);
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

        if let Some(resource) = state.core.world.resource_mut::<DeltaTime>() {
            resource.0 = dt;
        }
        if let Some(resource) = state.core.world.resource_mut::<ElapsedTime>() {
            resource.0 += dt;
        }

        queue_local_debug_inputs(&mut state.core.world);
        state.bus.run_frame(&mut state.core.world);
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
    bus.register(LoopPhase::Update, 5, FlagPickupSystem);
    bus.register(LoopPhase::Update, 10, FlagCarrySystem);
    bus.register(LoopPhase::Update, 20, WinConditionSystem);
    bus
}

fn setup_ctf_world(world: &mut World) {
    reload_scene(world, SCENE_PATH).unwrap_or_else(|err| {
        panic!("failed to load {SCENE_PATH}: {err}");
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
    let switch = keys.is_held(SWITCH_KEY);
    let restart = keys.is_held(RESTART_KEY);

    frames.push(InputFrame {
        tick: 0,
        player_id: 1,
        move_x: red_move_x,
        move_y: red_move_y,
        action_bits: (if keys.is_held(8) { ACTION_TAG } else { 0 })
            | (if switch { ACTION_SWITCH } else { 0 })
            | (if restart { ACTION_RESTART } else { 0 }),
        ctf_pointer: pointer_input,
    });
    frames.push(InputFrame {
        tick: 0,
        player_id: 2,
        move_x: blue_move_x,
        move_y: blue_move_y,
        action_bits: (if keys.is_held(9) { ACTION_TAG } else { 0 })
            | (if switch { ACTION_SWITCH } else { 0 })
            | (if restart { ACTION_RESTART } else { 0 }),
        ctf_pointer: pointer_input.map(|pointer| CtfPointerInput {
            aim_world: pointer.aim_world,
            click_world: None,
        }),
    });
    world.insert_resource(CtfInputState { frames });
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
    match code {
        KeyCode::Left => Some(0),
        KeyCode::Right => Some(1),
        KeyCode::Up => Some(2),
        KeyCode::Down => Some(3),
        KeyCode::W => Some(4),
        KeyCode::A => Some(5),
        KeyCode::S => Some(6),
        KeyCode::D => Some(7),
        KeyCode::Space => Some(8),
        KeyCode::Return => Some(9),
        KeyCode::LeftShift => Some(10),
        KeyCode::R => Some(11),
        _ => None,
    }
}

fn render(state: &mut CtfState) {
    state
        .core
        .render_ctx
        .sync_with_window(state.core.platform.window());

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

    {
        let ui = state.core.imgui.begin_frame(state.core.platform.window());
        draw_hud(ui, &state.core.world);
        state.core.imgui.end_frame(
            state.core.platform.window(),
            &state.core.render_ctx.device,
            &state.core.render_ctx.queue,
            &mut encoder,
            &view,
        );
    }

    state
        .core
        .render_ctx
        .queue
        .submit(std::iter::once(encoder.finish()));
    state.core.render_ctx.end_frame(surface_texture);
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

#[cfg(test)]
mod tests {
    use super::*;
    use forge_ecs::game::ctf::components::{Flag, PlayerMarker, Wall};
    use forge_ecs::game::ctf::resources::EntityRefs;

    #[test]
    fn setup_resolves_ctf_scene_entities() {
        let mut world = World::new();
        setup_ctf_world(&mut world);

        assert!(world.resource::<EntityRefs>().is_some());
        assert_eq!(world.query::<PlayerMarker>().count(), 4);
        assert_eq!(world.query::<Flag>().count(), 2);
        assert_eq!(world.query::<Wall>().count(), 16);
    }
}
