//! `GameRunner` — clean game loop with no editor UI.
//!
//! The loop sends `First → Update → Last` messages via the [`MessageBus`].
//! Systems subscribe at registration time and are called in priority order.

use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::components::{Color, Shape, Transform};
use crate::ecs::resource::{DeltaTime, ElapsedTime};
use crate::ecs::world::World;
use crate::messaging::MessageBus;
use crate::renderer::draw::DrawCommand;

use super::core::AppCore;

/// Runs the game without any editor UI.
///
/// Register systems via `runner.bus.register(phase, priority, system)` before
/// calling [`GameRunner::run`].
pub struct GameRunner {
    /// Message bus — register systems here before calling `run`.
    pub bus: MessageBus,
    last_time: Instant,
}

impl GameRunner {
    /// Create a runner with an empty message bus.
    pub fn new() -> Self {
        Self { bus: MessageBus::new(), last_time: Instant::now() }
    }

    /// Create the event loop, build the window inside `resumed`, and block
    /// until the window closes.
    pub fn run(
        self,
        title: &str,
        width: u32,
        height: u32,
        setup: impl FnOnce(&mut World) + 'static,
    ) {
        let event_loop = EventLoop::new().expect("failed to create event loop");
        let mut handler = GameHandle {
            runner: self,
            setup: Some(Box::new(setup)),
            title: title.to_string(),
            width,
            height,
            core: None,
        };
        event_loop.run_app(&mut handler).expect("event loop error");
    }

    // -----------------------------------------------------------------------

    fn update(&mut self, core: &mut AppCore) {
        let now = Instant::now();
        let dt  = now.duration_since(self.last_time).as_secs_f32();
        self.last_time = now;

        if let Some(r) = core.world.resource_mut::<DeltaTime>()  { r.0  = dt; }
        if let Some(r) = core.world.resource_mut::<ElapsedTime>() { r.0 += dt; }

        self.bus.run_frame(&mut core.world);
    }

    fn render(&mut self, core: &mut AppCore) {
        let w = core.render_ctx.surface_config.width  as f32;
        let h = core.render_ctx.surface_config.height as f32;

        // Queue scene draw commands (world-space → centred screen-space).
        core.world.query3::<Transform, Shape, Color>()
            .for_each(|(_, transform, shape, color)| {
                let cmd = make_draw_cmd(transform, shape, color, w, h);
                core.draw_queue.push(cmd);
            });

        let Some((surface_texture, view)) = core.render_ctx.begin_frame() else { return; };
        let mut encoder = core.render_ctx.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor { label: Some("game frame") },
        );

        core.draw_queue.flush(
            &core.render_ctx, &view, &mut encoder,
            &core.circle_pipeline, &core.rect_pipeline,
            [0.10, 0.10, 0.10, 1.0],
        );

        core.render_ctx.queue.submit(std::iter::once(encoder.finish()));
        core.render_ctx.end_frame(surface_texture);
    }
}

impl Default for GameRunner {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// ApplicationHandler impl
// ---------------------------------------------------------------------------

struct GameHandle {
    runner: GameRunner,
    setup:  Option<Box<dyn FnOnce(&mut World)>>,
    title:  String,
    width:  u32,
    height: u32,
    core:   Option<AppCore>,
}

impl ApplicationHandler for GameHandle {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.core.is_some() { return; }
        let attrs = WindowAttributes::default()
            .with_title(&self.title)
            .with_inner_size(PhysicalSize::new(self.width, self.height))
            .with_resizable(true);
        let window: Window = event_loop.create_window(attrs).expect("window creation failed");
        let mut core = AppCore::from_window(window).expect("AppCore creation failed");
        if let Some(setup) = self.setup.take() { setup(&mut core.world); }
        self.core = Some(core);
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        if let Some(core) = &mut self.core {
            let full = winit::event::Event::<()>::NewEvents(cause);
            core.imgui.handle_event(core.platform.window(), &full);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(core) = &mut self.core else { return; };
        if window_id != core.platform.window.id() { return; }

        core.imgui.handle_window_event(core.platform.window(), window_id, &event);

        match &event {
            WindowEvent::CloseRequested         => event_loop.exit(),
            WindowEvent::Resized(s)             => core.render_ctx.resize(s.width, s.height),
            WindowEvent::RedrawRequested        => self.runner.render(core),
            _                                   => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        let Some(core) = &mut self.core else { return; };
        core.imgui.handle_about_to_wait(core.platform.window());
        self.runner.update(core);
        core.platform.window.request_redraw();
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a `(Transform, Shape, Color)` triple to a centred `DrawCommand`.
///
/// The renderer's coordinate system has (0, 0) at top-left, but the demo
/// treats (0, 0) as the viewport centre.  This helper offsets by half the
/// viewport so that world-origin sits at screen-centre.
pub(crate) fn make_draw_cmd(
    transform: &Transform,
    shape: &Shape,
    color: &Color,
    viewport_w: f32,
    viewport_h: f32,
) -> DrawCommand {
    let cx = viewport_w  * 0.5;
    let cy = viewport_h * 0.5;
    match shape {
        Shape::Circle { radius } => DrawCommand::Circle {
            x:      transform.position.x + cx,
            y:      transform.position.y + cy,
            radius: *radius,
            color: [color.r, color.g, color.b, color.a],
        },
        Shape::Rect { width, height } => DrawCommand::Rect {
            x:      transform.position.x + cx,
            y:      transform.position.y + cy,
            width:  *width,
            height: *height,
            color: [color.r, color.g, color.b, color.a],
        },
    }
}
