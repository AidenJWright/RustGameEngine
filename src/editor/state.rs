//! Editor runtime state — selection, camera, scene file path, status messages,
//! and the component/system registries that power the inspector UI.

use super::camera::Camera2D;
use super::component_registry::{ComponentDescriptor, SystemComponentEntry};
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
    /// Component descriptors used to populate the "Add Component" dropdown.
    ///
    /// Populated once at editor startup (in `demo/editor.rs`).  Transform is
    /// excluded because it is guaranteed on every entity and cannot be added
    /// or removed via the inspector.
    pub component_registry: Vec<ComponentDescriptor>,
    /// Maps each system name to the component names it reads.
    ///
    /// Populated once at editor startup alongside `component_registry`.
    pub system_component_map: Vec<SystemComponentEntry>,
    /// Index into `component_registry` for the current "Add Component" combo selection.
    pub add_component_selection: usize,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            selected_entity: None,
            camera: Camera2D::new(),
            scene_path: "scene.json".to_string(),
            status_message: String::new(),
            component_registry: Vec::new(),
            system_component_map: Vec::new(),
            add_component_selection: 0,
        }
    }
}
