//! Shape component — describes the renderable primitive.

use crate::ecs::component::Component;

/// The geometric shape used for rendering.
#[derive(Debug, Clone)]
pub enum Shape {
    /// A filled circle.
    Circle { radius: f32 },
    /// An axis-aligned rectangle.
    Rect { width: f32, height: f32 },
}

impl Component for Shape {}
