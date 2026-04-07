//! Game-domain components.

pub mod camera;
pub mod color;
pub mod health;
pub mod shape;
pub mod sinusoid;
pub mod tag;
pub mod transform;
pub mod velocity;

pub use camera::Camera;
pub use color::Color;
pub use health::Health;
pub use shape::Shape;
pub use sinusoid::SinusoidComponent;
pub use tag::Tag;
pub use transform::Transform;
pub use velocity::Velocity;
