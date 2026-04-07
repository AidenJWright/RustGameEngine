//! Editor-specific types: 2D camera, runtime state, and component registry.

pub mod camera;
pub mod component_registry;
pub mod state;

pub use camera::Camera2D;
pub use component_registry::{ComponentDescriptor, SystemComponentEntry};
pub use state::EditorState;
