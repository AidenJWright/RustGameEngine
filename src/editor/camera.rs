//! 2D editor camera — pans and zooms the viewport.

use crate::math::Vec2;
use crate::renderer::draw::DrawCommand;

/// A 2D camera that converts world-space draw commands to screen-space.
///
/// World origin (0, 0) maps to `(viewport_w / 2, viewport_h / 2)` at zoom 1.
/// Panning moves the world origin; zooming scales distances from the centre.
#[derive(Debug, Clone)]
pub struct Camera2D {
    /// World-space position of the viewport centre.
    pub position: Vec2,
    /// Zoom multiplier (1.0 = 1:1, > 1 = closer, < 1 = farther).
    pub zoom: f32,
}

impl Default for Camera2D {
    fn default() -> Self {
        Self { position: Vec2::ZERO, zoom: 1.0 }
    }
}

impl Camera2D {
    /// Create a camera centred at the world origin at zoom 1.
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply this camera's transform to a draw command.
    ///
    /// Converts world-space coordinates to screen-space pixels, with
    /// `(0, 0)` mapping to `(viewport_w / 2, viewport_h / 2)`.
    pub fn transform_draw_cmd(
        &self,
        cmd: DrawCommand,
        viewport_w: f32,
        viewport_h: f32,
    ) -> DrawCommand {
        let cx = viewport_w * 0.5;
        let cy = viewport_h * 0.5;
        match cmd {
            DrawCommand::Circle { x, y, radius, color } => DrawCommand::Circle {
                x:      (x - self.position.x) * self.zoom + cx,
                y:      (y - self.position.y) * self.zoom + cy,
                radius: radius * self.zoom,
                color,
            },
            DrawCommand::Rect { x, y, width, height, color } => DrawCommand::Rect {
                x:      (x - self.position.x) * self.zoom + cx,
                y:      (y - self.position.y) * self.zoom + cy,
                width:  width  * self.zoom,
                height: height * self.zoom,
                color,
            },
        }
    }

    /// Pan the camera by `(dx, dy)` screen pixels.
    ///
    /// Divides by zoom so one screen-pixel always feels the same regardless
    /// of zoom level.
    pub fn pan(&mut self, dx: f32, dy: f32) {
        self.position.x -= dx / self.zoom;
        self.position.y -= dy / self.zoom;
    }

    /// Zoom toward or away from the viewport centre.
    ///
    /// `delta` is the scroll-wheel notch count (positive = zoom in).
    pub fn zoom_toward(&mut self, delta: f32) {
        self.zoom = (self.zoom * (1.0 + delta * 0.1)).clamp(0.05, 20.0);
    }
}
