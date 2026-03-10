//! Tag component — a string label for an entity.

use crate::ecs::component::Component;
use serde::{Deserialize, Serialize};

/// A human-readable label for an entity (e.g. `"player"`, `"scene_root"`).
///
/// Uses `String` so that editor-created tags and deserialized scenes work
/// without requiring `'static` string literals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag(pub String);

impl Tag {
    /// Convenience constructor from any string-like value.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// View the tag as a `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Component for Tag {}
