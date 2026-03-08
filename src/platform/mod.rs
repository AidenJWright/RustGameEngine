//! Platform abstraction — windowing and OS events.
//!
//! The engine only depends on `Platform` + `PlatformEvent`. Backend modules
//! are feature-gated so only the selected platform is compiled.

pub mod traits;

#[cfg(feature = "platform-windows")]
pub mod windows;

pub use traits::{KeyCode, MouseButton, Platform, PlatformError, PlatformEvent};

/// The concrete platform type selected at compile time.
#[cfg(feature = "platform-windows")]
pub type ActivePlatform = windows::WinitPlatform;
