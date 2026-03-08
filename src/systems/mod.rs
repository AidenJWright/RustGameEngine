//! Built-in engine systems.

pub mod debug;
pub mod health;
pub mod movement;
pub mod sinusoid;

pub use debug::DebugSystem;
pub use health::HealthSystem;
pub use movement::MovementSystem;
pub use sinusoid::{SinusoidComponent, SinusoidSystem};
