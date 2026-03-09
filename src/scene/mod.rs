//! Scene serialisation — JSON save/load for the ECS world.

pub mod data;

pub use data::{load_scene, save_scene, EntityData, SceneData};
