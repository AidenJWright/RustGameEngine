//! Re-exports `math::Transform` as an ECS `Component`.

pub use crate::math::Transform;
use crate::ecs::component::Component;

impl Component for Transform {}
