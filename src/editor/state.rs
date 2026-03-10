//! Editor runtime state — selection, camera, scene file path, status messages.

use super::camera::Camera2D;
use crate::ecs::entity::Entity;

/// Mutable editor runtime state passed between frames.
pub struct EditorState {
    /// Currently selected entity in the hierarchy panel.
    pub selected_entity: Option<Entity>,
    /// 2D viewport camera.
    pub camera: Camera2D,
    /// Path used for Save / Load scene operations.
    pub scene_path: String,
    /// One-line status message shown at the bottom of the editor window.
    pub status_message: String,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            selected_entity: None,
            camera: Camera2D::new(),
            scene_path: "scene.json".to_string(),
            status_message: String::new(),
        }
    }
}
