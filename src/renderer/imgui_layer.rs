//! imgui integration layer — wraps imgui-rs, imgui-wgpu, and imgui-winit-support.

use imgui::Context;
use imgui_wgpu::{Renderer, RendererConfig};
use imgui_winit_support::{HiDpiMode, WinitPlatform};
use wgpu::{CommandEncoder, Device, Queue, StoreOp, TextureFormat};
use winit::{event::Event, window::Window};

/// Holds all imgui state and provides a two-step frame API.
///
/// Usage per frame:
/// 1. `handle_event` for every OS event (feed before polling your own events).
/// 2. `begin_frame` → mutate UI via the returned `&mut imgui::Ui`.
/// 3. `end_frame` with the device/queue/encoder/view after scene draw.
pub struct ImguiLayer {
    ctx: Context,
    platform: WinitPlatform,
    renderer: Renderer,
}

impl ImguiLayer {
    /// Create the imgui layer.
    ///
    /// `format` must match the main surface attachment format so the imgui
    /// render pass writes to the correct attachment.
    pub fn new(window: &Window, device: &Device, queue: &Queue, format: TextureFormat) -> Self {
        let mut ctx = Context::create();
        ctx.set_ini_filename(None); // disable imgui.ini persistence

        let mut platform = WinitPlatform::new(&mut ctx);
        platform.attach_window(ctx.io_mut(), window, HiDpiMode::Default);

        let renderer_config = RendererConfig {
            texture_format: format,
            ..Default::default()
        };
        let renderer = Renderer::new(&mut ctx, device, queue, renderer_config);

        Self {
            ctx,
            platform,
            renderer,
        }
    }

    /// Forward an OS event to imgui's winit platform glue.
    pub fn handle_event(&mut self, window: &Window, event: &Event<()>) {
        self.platform.handle_event(self.ctx.io_mut(), window, event);
    }

    /// Forward a `WindowEvent` to imgui (for use with `ApplicationHandler`).
    pub fn handle_window_event(
        &mut self,
        window: &Window,
        window_id: winit::window::WindowId,
        event: &winit::event::WindowEvent,
    ) {
        let full = winit::event::Event::<()>::WindowEvent {
            window_id,
            event: event.clone(),
        };
        self.platform.handle_event(self.ctx.io_mut(), window, &full);
    }

    /// Notify imgui that all events for this frame have been processed.
    pub fn handle_about_to_wait(&mut self, window: &Window) {
        self.platform.handle_event(
            self.ctx.io_mut(),
            window,
            &winit::event::Event::<()>::AboutToWait,
        );
    }

    /// Begin an imgui frame and return a mutable UI handle.
    pub fn begin_frame<'a>(&'a mut self, window: &Window) -> &'a mut imgui::Ui {
        self.platform
            .prepare_frame(self.ctx.io_mut(), window)
            .expect("imgui prepare_frame failed");
        self.ctx.new_frame()
    }

    /// Render the built-up UI into `view` (runs after the scene pass).
    pub fn end_frame(
        &mut self,
        _window: &Window,
        device: &Device,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        view: &wgpu::TextureView,
    ) {
        // prepare_render is skipped — it only updates cursor icon, non-critical.
        let draw_data = self.ctx.render();

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("imgui pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load, // preserve scene geometry underneath
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        self.renderer
            .render(draw_data, queue, device, &mut render_pass)
            .expect("imgui render failed");
    }
}
