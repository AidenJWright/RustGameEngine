//! Re-exports `math::Transform` as an ECS `Component`.

use crate::ecs::component::Component;
pub use crate::math::Transform;

impl Component for Transform {}
