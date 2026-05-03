//! Editor runtime state — selection, camera, scene file path, status messages,
//! and the component/system registries that power the inspector UI.

use super::camera::Camera2D;
use super::component_registry::{ComponentDescriptor, SystemComponentEntry};
use crate::ecs::entity::Entity;
use crate::math::Vec2;

/// Stored normal-mode editor window rectangle used when restoring from maximize.
#[derive(Debug, Clone, Copy)]
pub struct EditorWindowRect {
    pub pos: [f32; 2],
    pub size: [f32; 2],
}

/// Current left-button scene drag operation.
#[derive(Debug, Clone, Copy)]
pub struct SceneEntityDrag {
    pub entity: Entity,
    pub grab_offset: Vec2,
}

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
    /// Whether the unified editor window should fill the viewport.
    pub editor_window_maximized: bool,
    /// Last normal window rectangle captured before maximizing.
    pub editor_window_restore_rect: Option<EditorWindowRect>,
    /// Force the stored normal rectangle back into imgui on the next frame.
    pub editor_window_restore_pending: bool,
    /// Entity currently being moved directly in the scene viewport.
    pub scene_drag: Option<SceneEntityDrag>,
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
            editor_window_maximized: false,
            editor_window_restore_rect: None,
            editor_window_restore_pending: false,
            scene_drag: None,
        }
    }
}
