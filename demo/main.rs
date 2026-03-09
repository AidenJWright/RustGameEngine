//! Forge ECS demo — sinusoidal shapes + imgui live-editing.
//!
//! Run with: `cargo run --bin demo`

#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::too_many_lines)]

use std::f32::consts::PI;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use forge_ecs::components::{Color, Shape, Tag, Transform};
use forge_ecs::ecs::command_buffer::CommandBuffer;
use forge_ecs::ecs::entity::Entity;
use forge_ecs::ecs::resource::{DeltaTime, ElapsedTime};
use forge_ecs::ecs::system::System;
use forge_ecs::ecs::world::World;
use forge_ecs::math::Vec3;
use forge_ecs::platform::WinitPlatform;
use forge_ecs::renderer::context::RenderContext;
use forge_ecs::renderer::draw::{DrawCommand, DrawQueue};
use forge_ecs::renderer::imgui_layer::ImguiLayer;
use forge_ecs::renderer::{CirclePipeline, RectPipeline};
use forge_ecs::systems::sinusoid::SinusoidComponent;
use forge_ecs::systems::SinusoidSystem;

// ---------------------------------------------------------------------------
// Runtime state — everything that lives once the window is open
// ---------------------------------------------------------------------------

struct DemoState {
    platform:        WinitPlatform,
    render_ctx:      RenderContext,
    circle_pipeline: CirclePipeline,
    rect_pipeline:   RectPipeline,
    draw_queue:      DrawQueue,
    imgui:           ImguiLayer,
    world:           World,
    circle_entity:   Entity,
    rect_entity:     Entity,
    last_time:       Instant,
    sinusoid_system: SinusoidSystem,
}

// ---------------------------------------------------------------------------
// ApplicationHandler
// ---------------------------------------------------------------------------

struct DemoApp {
    state: Option<DemoState>,
}

impl ApplicationHandler for DemoApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() { return; }

        let attrs = WindowAttributes::default()
            .with_title("Forge ECS Demo")
            .with_inner_size(PhysicalSize::new(1280_u32, 720_u32))
            .with_resizable(true);
        let window: Window = event_loop.create_window(attrs).expect("failed to create window");

        let (width, height) = {
            let s = window.inner_size();
            (s.width, s.height)
        };

        let render_ctx = RenderContext::new(&window, width, height)
            .expect("failed to create render context");

        let circle_pipeline = CirclePipeline::new(&render_ctx.device, render_ctx.surface_format);
        let rect_pipeline   = RectPipeline::new(&render_ctx.device, render_ctx.surface_format);
        let draw_queue      = DrawQueue::new();

        let imgui = ImguiLayer::new(
            &window,
            &render_ctx.device,
            &render_ctx.queue,
            render_ctx.surface_format,
        );

        let mut world = World::new();
        world.insert_resource(DeltaTime(0.0));
        world.insert_resource(ElapsedTime(0.0));

        // Scene setup
        let scene_root = world.spawn();
        world.insert(scene_root, Tag::new("scene_root"));

        let circle_entity = world.spawn_child(scene_root);
        world.insert(circle_entity, Transform {
            position: Vec3::new(800.0, 0.0, 0.0),
            ..Transform::identity()
        });
        world.insert(circle_entity, Shape::Circle { radius: 50.0 });
        world.insert(circle_entity, Color { r: 1.0, g: 0.4, b: 0.1, a: 1.0 });
        world.insert(circle_entity, SinusoidComponent {
            amplitude: 150.0,
            frequency: 1.0,
            phase: 0.0,
            base_y: 0.0,
        });

        let rect_entity = world.spawn_child(scene_root);
        world.insert(rect_entity, Transform {
            position: Vec3::new(400.0, 0.0, 0.0),
            ..Transform::identity()
        });
        world.insert(rect_entity, Shape::Rect { width: 100.0, height: 100.0 });
        world.insert(rect_entity, Color { r: 0.2, g: 0.6, b: 1.0, a: 1.0 });
        world.insert(rect_entity, SinusoidComponent {
            amplitude: 150.0,
            frequency: 1.0,
            phase: PI / 2.0,
            base_y: 0.0,
        });

        // Scene tree assertions + print
        assert_eq!(world.scene_tree().children(scene_root).len(), 2);
        assert_eq!(world.scene_tree().parent(circle_entity), Some(scene_root));
        assert_eq!(world.scene_tree().parent(rect_entity), Some(scene_root));

        println!("=== Scene Tree ===");
        world.scene_tree().walk_depth_first(scene_root, |entity, depth| {
            let indent = "  ".repeat(depth);
            let tag = world.get::<Tag>(entity).map(|t| t.as_str()).unwrap_or("<no tag>");
            println!("{indent}{entity} [{tag}]");
        });
        println!("==================");

        let platform = WinitPlatform::from_window(window);

        self.state = Some(DemoState {
            platform,
            render_ctx,
            circle_pipeline,
            rect_pipeline,
            draw_queue,
            imgui,
            world,
            circle_entity,
            rect_entity,
            last_time: Instant::now(),
            sinusoid_system: SinusoidSystem,
        });
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        if let Some(s) = &mut self.state {
            let full = winit::event::Event::<()>::NewEvents(cause);
            s.imgui.handle_event(s.platform.window(), &full);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(s) = &mut self.state else { return; };
        if window_id != s.platform.window.id() { return; }

        s.imgui.handle_window_event(s.platform.window(), window_id, &event);

        match &event {
            WindowEvent::CloseRequested  => event_loop.exit(),
            WindowEvent::Resized(sz)     => s.render_ctx.resize(sz.width, sz.height),
            WindowEvent::RedrawRequested => render(s),
            _                            => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        let Some(s) = &mut self.state else { return; };
        s.imgui.handle_about_to_wait(s.platform.window());
        update(s);
        s.platform.window.request_redraw();
    }
}

// ---------------------------------------------------------------------------
// Update & render
// ---------------------------------------------------------------------------

fn update(s: &mut DemoState) {
    let now = Instant::now();
    let dt  = now.duration_since(s.last_time).as_secs_f32();
    s.last_time = now;

    if let Some(r) = s.world.resource_mut::<DeltaTime>()  { r.0  = dt; }
    if let Some(r) = s.world.resource_mut::<ElapsedTime>() { r.0 += dt; }

    let mut cmds = CommandBuffer::new();
    s.sinusoid_system.run(&s.world, &mut cmds);
    cmds.flush(&mut s.world);

    // Queue draw commands for next frame.
    s.world.query3::<Transform, Shape, Color>()
        .for_each(|(_, transform, shape, color)| {
            let cmd = match shape {
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
            };
            s.draw_queue.push(cmd);
        });
}

fn render(s: &mut DemoState) {
    let Some((surface_texture, view)) = s.render_ctx.begin_frame() else { return; };

    let mut encoder = s.render_ctx.device.create_command_encoder(
        &wgpu::CommandEncoderDescriptor { label: Some("frame encoder") },
    );

    s.draw_queue.flush(
        &s.render_ctx,
        &view,
        &mut encoder,
        &s.circle_pipeline,
        &s.rect_pipeline,
        [0.15, 0.15, 0.15, 1.0],
    );

    // imgui pass (after scene so UI is on top).
    {
        let ui = s.imgui.begin_frame(s.platform.window());
        let circle_entity = s.circle_entity;
        let rect_entity   = s.rect_entity;

        ui.window("Entity Colors")
            .size([320.0, 220.0], imgui::Condition::FirstUseEver)
            .build(|| {
                if let Some(color) = s.world.get_mut::<Color>(circle_entity) {
                    let mut c = [color.r, color.g, color.b, color.a];
                    if ui.color_edit4("Circle Color", &mut c) {
                        color.r = c[0]; color.g = c[1];
                        color.b = c[2]; color.a = c[3];
                    }
                }
                if let Some(sin) = s.world.get_mut::<SinusoidComponent>(circle_entity) {
                    ui.slider("Circle Freq", 0.1_f32, 5.0, &mut sin.frequency);
                    ui.slider("Circle Amp",  10.0_f32, 400.0, &mut sin.amplitude);
                }
                ui.separator();
                if let Some(color) = s.world.get_mut::<Color>(rect_entity) {
                    let mut c = [color.r, color.g, color.b, color.a];
                    if ui.color_edit4("Rect Color", &mut c) {
                        color.r = c[0]; color.g = c[1];
                        color.b = c[2]; color.a = c[3];
                    }
                }
                if let Some(sin) = s.world.get_mut::<SinusoidComponent>(rect_entity) {
                    ui.slider("Rect Freq", 0.1_f32, 5.0, &mut sin.frequency);
                    ui.slider("Rect Amp",  10.0_f32, 400.0, &mut sin.amplitude);
                }
            });

        s.imgui.end_frame(
            s.platform.window(),
            &s.render_ctx.device,
            &s.render_ctx.queue,
            &mut encoder,
            &view,
        );
    }

    s.render_ctx.queue.submit(std::iter::once(encoder.finish()));
    s.render_ctx.end_frame(surface_texture);
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut app = DemoApp { state: None };
    event_loop.run_app(&mut app).expect("event loop error");
}
