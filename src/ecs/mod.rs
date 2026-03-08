//! ECS core — entity, component, world, scene tree, systems, resources.

pub mod command_buffer;
pub mod component;
pub mod entity;
pub mod resource;
pub mod scene_tree;
pub mod system;
pub mod world;

pub use command_buffer::CommandBuffer;
pub use component::{Component, ComponentRegistry, ComponentStorage};
pub use entity::{Entity, EntityAllocator};
pub use resource::{DeltaTime, ElapsedTime, Resource, Resources};
pub use scene_tree::SceneTree;
pub use system::{FnSystem, Scheduler, System};
pub use world::World;
