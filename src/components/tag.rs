//! Tag component — a static string label.

use crate::ecs::component::Component;

/// A human-readable label for an entity (e.g. `"player"`, `"scene_root"`).
#[derive(Debug, Clone)]
pub struct Tag(pub &'static str);

impl Component for Tag {}
