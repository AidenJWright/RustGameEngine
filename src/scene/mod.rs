//! Scene serialisation — JSON save/load for the ECS world.

pub mod data;

pub use data::{clear_world, load_scene, reload_scene, save_scene, EntityData, SceneData};
