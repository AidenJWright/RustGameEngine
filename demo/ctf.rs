//! Local two-player capture-the-flag game.
//!
//! Run with: `cargo run --bin ctf`

#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::too_many_lines)]

use std::f32::consts::FRAC_PI_2;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use forge_ecs::app::AppCore;
use forge_ecs::components::{
    Color, ConfigKey, InputAxis, PlayerInput, Shape, Tag, Transform, Velocity,
};
use forge_ecs::ecs::entity::Entity;
use forge_ecs::ecs::resource::{DeltaTime, ElapsedTime, KeysPressed};
use forge_ecs::ecs::world::World;
use forge_ecs::game::ctf::components::{Flag, PlayerMarker, Wall};
use forge_ecs::game::ctf::resources::{CarrierState, EntityRefs, GameState, TagInputState};
use forge_ecs::game::ctf::systems::hud::draw_hud;
use forge_ecs::game::ctf::systems::{
    FlagCarrySystem, FlagPickupSystem, StopOnWinSystem, TaggingSystem, WallCollisionSystem,
    WinConditionSystem,
};
use forge_ecs::game::ctf::{ARENA_HEIGHT, ARENA_WIDTH, PLAYER_SPEED};
use forge_ecs::math::Vec3;
use forge_ecs::messaging::{LoopPhase, MessageBus};
use forge_ecs::platform::{map_window_event, KeyCode, PlatformEvent};
use forge_ecs::renderer::draw::DrawCommand;
use forge_ecs::scene::reload_scene;
use forge_ecs::systems::{MovementSystem, PlayerInputSystem};

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

        state.bus.run_frame(&mut state.core.world);
        state.core.platform.window.request_redraw();
    }
}

fn build_bus() -> MessageBus {
    let mut bus = MessageBus::new();
    bus.register(
        LoopPhase::Update,
        PlayerInputSystem::PRIORITY,
        PlayerInputSystem,
    );
    bus.register(LoopPhase::Update, -9, StopOnWinSystem);
    bus.register(LoopPhase::Update, MovementSystem::PRIORITY, MovementSystem);
    bus.register(LoopPhase::Update, 2, WallCollisionSystem);
    bus.register(LoopPhase::Update, 5, FlagPickupSystem);
    bus.register(LoopPhase::Update, 10, FlagCarrySystem);
    bus.register(LoopPhase::Update, 15, TaggingSystem);
    bus.register(LoopPhase::Update, 20, WinConditionSystem);
    bus
}

fn setup_ctf_world(world: &mut World) {
    reload_scene(world, SCENE_PATH).unwrap_or_else(|err| {
        panic!("failed to load {SCENE_PATH}: {err}");
    });

    world.insert_resource(KeysPressed::default());
    world.insert_resource(GameState::default());
    world.insert_resource(CarrierState::default());
    world.insert_resource(TagInputState::default());

    let p1_flag = find_tag(world, "p1_flag");
    let p2_flag = find_tag(world, "p2_flag");
    let p1_spawn_entity = find_tag(world, "p1_spawn");
    let p2_spawn_entity = find_tag(world, "p2_spawn");

    world.insert(p1_flag, Flag { owner_id: 1 });
    world.insert(p2_flag, Flag { owner_id: 2 });
    attach_wall_components(world);

    let p1_spawn = transform_xy(world, p1_spawn_entity);
    let p2_spawn = transform_xy(world, p2_spawn_entity);
    let p1_flag_spawn = transform_xy(world, p1_flag);
    let p2_flag_spawn = transform_xy(world, p2_flag);

    let p1 = spawn_player(
        world,
        1,
        "p1",
        p1_spawn,
        [1.0, 0.18, 0.18, 1.0],
        FRAC_PI_2,
        PlayerInput {
            horizontal: InputAxis::wasd_horizontal(),
            vertical: InputAxis::wasd_vertical(),
            speed: PLAYER_SPEED,
        },
    );
    let p2 = spawn_player(
        world,
        2,
        "p2",
        p2_spawn,
        [0.18, 0.44, 1.0, 1.0],
        -FRAC_PI_2,
        PlayerInput {
            horizontal: InputAxis {
                negative: ConfigKey::ArrowLeft,
                positive: ConfigKey::ArrowRight,
            },
            vertical: InputAxis {
                negative: ConfigKey::ArrowUp,
                positive: ConfigKey::ArrowDown,
            },
            speed: PLAYER_SPEED,
        },
    );

    world.insert_resource(EntityRefs {
        p1,
        p2,
        p1_flag,
        p2_flag,
        p1_spawn,
        p2_spawn,
        p1_flag_spawn,
        p2_flag_spawn,
    });
}

fn spawn_player(
    world: &mut World,
    id: u8,
    tag: &str,
    spawn: (f32, f32),
    color: [f32; 4],
    rotation: f32,
    input: PlayerInput,
) -> Entity {
    let entity = world.spawn();
    world.insert(entity, Tag::new(tag));
    world.insert(
        entity,
        Transform {
            position: Vec3::new(spawn.0, spawn.1, 10.0),
            rotation,
            ..Transform::identity()
        },
    );
    world.insert(entity, Shape::Triangle { size: 40.0 });
    world.insert(
        entity,
        Color {
            r: color[0],
            g: color[1],
            b: color[2],
            a: color[3],
        },
    );
    world.insert(entity, Velocity { dx: 0.0, dy: 0.0 });
    world.insert(entity, input);
    world.insert(entity, PlayerMarker { id });
    entity
}

fn attach_wall_components(world: &mut World) {
    let wall_entities: Vec<Entity> = world
        .query::<Tag>()
        .filter(|(_, tag)| tag.as_str() == "wall")
        .map(|(entity, _)| entity)
        .collect();

    for entity in wall_entities {
        if let Some(Shape::Rect { width, height }) = world.get::<Shape>(entity).cloned() {
            world.insert(
                entity,
                Wall {
                    w: width * 0.5,
                    h: height * 0.5,
                },
            );
        }
    }
}

fn find_tag(world: &World, needle: &str) -> Entity {
    world
        .query::<Tag>()
        .find(|(_, tag)| tag.as_str() == needle)
        .map(|(entity, _)| entity)
        .unwrap_or_else(|| panic!("missing required scene tag `{needle}`"))
}

fn transform_xy(world: &World, entity: Entity) -> (f32, f32) {
    let transform = world
        .get::<Transform>(entity)
        .unwrap_or_else(|| panic!("entity {entity} is missing Transform"));
    (transform.position.x, transform.position.y)
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

    #[test]
    fn setup_resolves_ctf_scene_entities() {
        let mut world = World::new();
        setup_ctf_world(&mut world);

        assert!(world.resource::<EntityRefs>().is_some());
        assert_eq!(world.query::<PlayerMarker>().count(), 2);
        assert_eq!(world.query::<Flag>().count(), 2);
        assert_eq!(world.query::<Wall>().count(), 16);
    }
}
