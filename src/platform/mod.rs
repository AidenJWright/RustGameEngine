//! Cross-platform platform layer for windowing and input.
//!
//! The engine currently uses a single `winit` backend that works on
//! Windows, macOS, and Linux.

pub mod winit;

pub use winit::{
    map_mouse_button, map_physical_key, map_window_event, KeyCode, MouseButton, PlatformError,
    PlatformEvent, WinitPlatform,
};
