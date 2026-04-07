//! Component registry for the editor — drives the "Add Component" dropdown
//! and the system-usage display in the inspector.
//!
//! Populate [`ComponentDescriptor`]s at editor startup (in `demo/editor.rs`)
//! and store them in [`EditorState::component_registry`].

use crate::ecs::entity::Entity;
use crate::ecs::world::World;

/// Describes one component type to the editor UI.
///
/// Create one descriptor per component and register it with
/// [`EditorState::component_registry`] before the event loop starts.
pub struct ComponentDescriptor {
    /// Display name shown in the "Add Component" dropdown.
    pub name: &'static str,
    /// Returns `true` when `entity` already has this component.
    pub has: fn(&World, Entity) -> bool,
    /// Inserts a default instance of this component onto `entity`.
    pub add: fn(&mut World, Entity),
    /// Removes this component from `entity`.
    pub remove: fn(&mut World, Entity),
}

/// Maps system names to the component names they read.
///
/// Used by the inspector to show which systems depend on each component.
///
/// Example entry: `("SinusoidSystem", &["Transform", "SinusoidComponent"])`
pub struct SystemComponentEntry {
    pub system_name: &'static str,
    pub component_names: &'static [&'static str],
}
