//! `AppCore` — shared infrastructure owned by both runners.

use crate::ecs::resource::{DeltaTime, ElapsedTime};
use crate::ecs::world::World;
use crate::platform::WinitPlatform;
use crate::renderer::context::RenderContext;
use crate::renderer::draw::DrawQueue;
use crate::renderer::imgui_layer::ImguiLayer;
use crate::renderer::{CirclePipeline, RectPipeline};

/// Owns every piece of shared infrastructure for one window.
///
/// Both [`super::game_runner::GameRunner`] and
/// [`super::editor_runner::EditorRunner`] operate on an `AppCore`.
pub struct AppCore {
    pub world:           World,
    pub platform:        WinitPlatform,
    pub render_ctx:      RenderContext,
    pub circle_pipeline: CirclePipeline,
    pub rect_pipeline:   RectPipeline,
    pub draw_queue:      DrawQueue,
    pub imgui:           ImguiLayer,
}

impl AppCore {
    /// Initialise the renderer and ECS world from an already-created `Window`.
    ///
    /// Call this inside `ApplicationHandler::resumed` after creating the window
    /// via `ActiveEventLoop::create_window`.
    ///
    /// # Errors
    /// Returns an error if the GPU context cannot be created.
    pub fn from_window(window: winit::window::Window) -> Result<Self, Box<dyn std::error::Error>> {
        let platform = WinitPlatform::from_window(window);
        let (w, h) = platform.inner_size();
        let render_ctx = RenderContext::new(platform.window(), w, h)?;

        let circle_pipeline = CirclePipeline::new(&render_ctx.device, render_ctx.surface_format);
        let rect_pipeline   = RectPipeline::new(&render_ctx.device, render_ctx.surface_format);
        let draw_queue = DrawQueue::new();

        let imgui = ImguiLayer::new(
            platform.window(),
            &render_ctx.device,
            &render_ctx.queue,
            render_ctx.surface_format,
        );

        let mut world = World::new();
        world.insert_resource(DeltaTime(0.0));
        world.insert_resource(ElapsedTime(0.0));

        Ok(Self { world, platform, render_ctx, circle_pipeline, rect_pipeline, draw_queue, imgui })
    }
}
