//! Scene serialisation — JSON save/load for the ECS world.

pub mod data;

/// Default editor-authored scene used by the editor and default game modes.
pub const DEFAULT_SCENE_PATH: &str = "assets/scene.json";

pub use data::{clear_world, load_scene, reload_scene, save_scene, EntityData, SceneData};
