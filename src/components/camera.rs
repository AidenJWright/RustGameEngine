//! 2D game camera component — pans and zooms the viewport.

use serde::{Deserialize, Serialize};

use crate::ecs::component::Component;
use crate::math::Vec2;
use crate::renderer::draw::DrawCommand;

/// A 2D camera that converts world-space draw commands to screen-space.
///
/// World origin `(0, 0)` maps to the screen origin at zoom 1.
/// Panning moves the world origin; zooming scales distances from it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Camera {
    /// World-space position of the viewport origin.
    pub position: Vec2,
    /// Zoom multiplier (1.0 = 1:1, > 1 = closer, < 1 = farther).
    pub zoom: f32,
}

impl Component for Camera {}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

impl Camera {
    /// Create a camera at the world origin at zoom 1.
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply this camera's transform to a draw command.
    ///
    /// World origin `(0,0)` maps to the centre of the viewport area so shapes
    /// are visible regardless of which panels are docked around the viewport.
    pub fn transform_draw_cmd(
        &self,
        cmd: DrawCommand,
        viewport_w: f32,
        viewport_h: f32,
    ) -> DrawCommand {
        // World origin (0,0) maps to the centre of the viewport area.
        let cx = viewport_w * 0.5;
        let cy = viewport_h * 0.5;
        match cmd {
            DrawCommand::Circle {
                x,
                y,
                radius,
                color,
            } => DrawCommand::Circle {
                x: cx + (x - self.position.x) * self.zoom,
                y: cy + (y - self.position.y) * self.zoom,
                radius: radius * self.zoom,
                color,
            },
            DrawCommand::Rect {
                x,
                y,
                width,
                height,
                color,
            } => DrawCommand::Rect {
                x: cx + (x - self.position.x) * self.zoom,
                y: cy + (y - self.position.y) * self.zoom,
                width: width * self.zoom,
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

    /// Zoom toward or away from the viewport origin.
    ///
    /// `delta` is the scroll-wheel notch count (positive = zoom in).
    pub fn zoom_toward(&mut self, delta: f32) {
        self.zoom = (self.zoom * (1.0 + delta * 0.1)).clamp(0.05, 20.0);
    }
}
