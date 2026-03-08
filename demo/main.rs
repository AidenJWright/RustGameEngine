//! Forge ECS demo — sinusoidal shapes + imgui live-editing.
//!
//! Run with: `cargo run --bin demo`

#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::too_many_lines)]

use std::f32::consts::PI;
use std::time::Instant;

use winit::event::{Event, WindowEvent};
use winit::event_loop::ControlFlow;

use forge_ecs::components::{Color, Shape, Tag, Transform};
use forge_ecs::ecs::command_buffer::CommandBuffer;
use forge_ecs::ecs::resource::{DeltaTime, ElapsedTime};
use forge_ecs::ecs::system::System;
use forge_ecs::ecs::world::World;
use forge_ecs::math::Vec3;
use forge_ecs::platform::{map_window_event, PlatformEvent, WinitPlatform};
use forge_ecs::renderer::context::RenderContext;
use forge_ecs::renderer::draw::{DrawCommand, DrawQueue};
use forge_ecs::renderer::imgui_layer::ImguiLayer;
use forge_ecs::renderer::{CirclePipeline, RectPipeline};
use forge_ecs::systems::sinusoid::SinusoidComponent;
use forge_ecs::systems::SinusoidSystem;

fn main() {
    // -----------------------------------------------------------------------
    // 1. Platform + window
    // -----------------------------------------------------------------------
    let mut platform = WinitPlatform::create_window("Forge ECS Demo", 1280, 720)
        .expect("failed to create window");

    let (width, height) = platform.inner_size();

    // -----------------------------------------------------------------------
    // 2. Render context (wgpu)
    // -----------------------------------------------------------------------
    let mut render_ctx = RenderContext::new(platform.window(), width, height)
        .expect("failed to create render context");

    // -----------------------------------------------------------------------
    // 3. Pipelines + draw queue
    // -----------------------------------------------------------------------
    let circle_pipeline = CirclePipeline::new(&render_ctx.device, render_ctx.surface_format);
    let rect_pipeline = RectPipeline::new(&render_ctx.device, render_ctx.surface_format);
    let mut draw_queue = DrawQueue::new();

    // -----------------------------------------------------------------------
    // 4. imgui layer
    // -----------------------------------------------------------------------
    let mut imgui = ImguiLayer::new(
        platform.window(),
        &render_ctx.device,
        &render_ctx.queue,
        render_ctx.surface_format,
    );

    // -----------------------------------------------------------------------
    // 5. ECS World + scene setup
    // -----------------------------------------------------------------------
    let mut world = World::new();

    world.insert_resource(DeltaTime(0.0));
    world.insert_resource(ElapsedTime(0.0));

    // Root scene entity.
    let scene_root = world.spawn();
    world.insert(scene_root, Tag("scene_root"));

    // Circle child.
    let circle_entity = world.spawn_child(scene_root);
    world.insert(circle_entity, Transform {
        position: Vec3::new(-200.0, 0.0, 0.0),
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

    // Rect child.
    let rect_entity = world.spawn_child(scene_root);
    world.insert(rect_entity, Transform {
        position: Vec3::new(200.0, 0.0, 0.0),
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

    // -----------------------------------------------------------------------
    // 6. Scene tree assertions + print
    // -----------------------------------------------------------------------
    assert_eq!(world.scene_tree().children(scene_root).len(), 2);
    assert_eq!(world.scene_tree().parent(circle_entity), Some(scene_root));
    assert_eq!(world.scene_tree().parent(rect_entity), Some(scene_root));

    println!("=== Scene Tree ===");
    world.scene_tree().walk_depth_first(scene_root, |entity, depth| {
        let indent = "  ".repeat(depth);
        let tag = world.get::<Tag>(entity).map(|t| t.0).unwrap_or("<no tag>");
        println!("{indent}{entity} [{tag}]");
    });
    println!("==================");

    // -----------------------------------------------------------------------
    // 7. Game loop — winit 0.30 style
    // -----------------------------------------------------------------------
    let event_loop = platform
        .take_event_loop()
        .expect("event loop already consumed");

    let sinusoid_system = SinusoidSystem;
    let mut last_time = Instant::now();

    #[allow(deprecated)]
    event_loop.run(move |event, elwt| {
        // Default to Poll so we redraw continuously.
        elwt.set_control_flow(ControlFlow::Poll);

        // Forward every event to imgui before we inspect it.
        imgui.handle_event(platform.window(), &event);

        match &event {
            Event::WindowEvent { event: win_event, window_id }
                if *window_id == platform.window().id() =>
            {
                // Map winit event to engine event.
                if let Some(platform_event) = map_window_event(win_event) {
                    match platform_event {
                        PlatformEvent::Quit => elwt.exit(),
                        PlatformEvent::Resized(w, h) => render_ctx.resize(w, h),
                        _ => {}
                    }
                }

                // Redraw is a WindowEvent in winit 0.30.
                if matches!(win_event, WindowEvent::RedrawRequested) {
                    let Some((surface_texture, view)) = render_ctx.begin_frame() else {
                        return;
                    };

                    let mut encoder = render_ctx.device.create_command_encoder(
                        &wgpu::CommandEncoderDescriptor { label: Some("frame encoder") },
                    );

                    // Scene pass.
                    draw_queue.flush(
                        &render_ctx,
                        &view,
                        &mut encoder,
                        &circle_pipeline,
                        &rect_pipeline,
                        [0.15, 0.15, 0.15, 1.0],
                    );

                    // imgui pass (after scene so UI is on top).
                    {
                        let ui = imgui.begin_frame(platform.window());

                        ui.window("Entity Colors")
                            .size([320.0, 220.0], imgui::Condition::FirstUseEver)
                            .build(|| {
                                if let Some(color) = world.get_mut::<Color>(circle_entity) {
                                    let mut c = [color.r, color.g, color.b, color.a];
                                    if ui.color_edit4("Circle Color", &mut c) {
                                        color.r = c[0]; color.g = c[1];
                                        color.b = c[2]; color.a = c[3];
                                    }
                                }
                                if let Some(sin) = world.get_mut::<SinusoidComponent>(circle_entity) {
                                    ui.slider("Circle Freq", 0.1_f32, 5.0, &mut sin.frequency);
                                    ui.slider("Circle Amp",  10.0_f32, 400.0, &mut sin.amplitude);
                                }
                                ui.separator();
                                if let Some(color) = world.get_mut::<Color>(rect_entity) {
                                    let mut c = [color.r, color.g, color.b, color.a];
                                    if ui.color_edit4("Rect Color", &mut c) {
                                        color.r = c[0]; color.g = c[1];
                                        color.b = c[2]; color.a = c[3];
                                    }
                                }
                                if let Some(sin) = world.get_mut::<SinusoidComponent>(rect_entity) {
                                    ui.slider("Rect Freq", 0.1_f32, 5.0, &mut sin.frequency);
                                    ui.slider("Rect Amp",  10.0_f32, 400.0, &mut sin.amplitude);
                                }
                            });

                        imgui.end_frame(
                            platform.window(),
                            &render_ctx.device,
                            &render_ctx.queue,
                            &mut encoder,
                            &view,
                        );
                    }

                    render_ctx.queue.submit(std::iter::once(encoder.finish()));
                    render_ctx.end_frame(surface_texture);
                }
            }

            // AboutToWait replaces MainEventsCleared in winit 0.30.
            Event::AboutToWait => {
                // Update time resources.
                let now = Instant::now();
                let dt = now.duration_since(last_time).as_secs_f32();
                last_time = now;

                if let Some(r) = world.resource_mut::<DeltaTime>() { r.0 = dt; }
                if let Some(r) = world.resource_mut::<ElapsedTime>() { r.0 += dt; }

                // Run sinusoid system.
                let mut cmds = CommandBuffer::new();
                sinusoid_system.run(&world, &mut cmds);
                cmds.flush(&mut world);

                // Queue draw commands for next frame.
                world.query3::<Transform, Shape, Color>()
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
                        draw_queue.push(cmd);
                    });

                platform.window().request_redraw();
            }

            _ => {}
        }
    }).expect("event loop error");
}
