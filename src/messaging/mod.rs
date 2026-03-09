//! Message-based game loop dispatch.
//!
//! The loop sends `LoopPhase` messages; systems subscribe via [`MessageBus`].

pub mod bus;
pub mod message;

pub use bus::MessageBus;
pub use message::LoopPhase;
