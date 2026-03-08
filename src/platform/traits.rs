//! Platform abstraction trait — decouples windowing backends from engine logic.
//!
//! To add a second platform (e.g. SDL2 on Linux):
//!   1. Create `platform/sdl2.rs` and define `struct Sdl2Platform`.
//!   2. Implement `Platform` for it, mapping SDL2 events to `PlatformEvent`.
//!   3. Feature-gate it with `#[cfg(feature = "platform-linux")]`.
//!
//! The rest of the engine only ever sees `dyn Platform` or a generic `P: Platform`.

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

/// Error type for platform creation.
#[derive(Debug)]
pub enum PlatformError {
    /// The OS refused to create a window.
    WindowCreation(String),
    /// Event loop could not be initialised.
    EventLoop(String),
}

impl std::fmt::Display for PlatformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WindowCreation(s) => write!(f, "window creation failed: {s}"),
            Self::EventLoop(s) => write!(f, "event loop error: {s}"),
        }
    }
}

impl std::error::Error for PlatformError {}

// ---------------------------------------------------------------------------
// Engine-native input types
// ---------------------------------------------------------------------------

/// Engine-native key code — maps from whatever the backend provides.
///
/// A second platform implementor maps its own key enum to these variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    Escape,
    Space,
    Return,
    Left,
    Right,
    Up,
    Down,
    /// Any key not explicitly listed above.
    Other(u32),
}

/// Mouse button identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u16),
}

// ---------------------------------------------------------------------------
// Platform events
// ---------------------------------------------------------------------------

/// Engine-native event emitted by [`Platform::poll_events`].
///
/// Every `Platform` implementation must map its OS events to these variants.
/// This is the **only** event type the rest of the engine handles.
#[derive(Debug, Clone)]
pub enum PlatformEvent {
    /// The user requested the application to close.
    Quit,
    /// The window was resized to the given physical pixel dimensions.
    Resized(u32, u32),
    /// The OS requested a redraw.
    RedrawRequested,
    /// A keyboard key was pressed.
    KeyPressed(KeyCode),
    /// A keyboard key was released.
    KeyReleased(KeyCode),
    /// The cursor moved to the given position in logical pixels.
    MouseMoved { x: f64, y: f64 },
    /// A mouse button state changed.
    MouseButton { button: MouseButton, pressed: bool },
}

// ---------------------------------------------------------------------------
// Platform trait
// ---------------------------------------------------------------------------

/// Abstraction over a windowing system backend.
///
/// ## Implementing a new platform
///
/// 1. Create `platform/<name>.rs`, define a struct (e.g. `Sdl2Platform`).
/// 2. Implement all methods below, translating OS events into `PlatformEvent`.
/// 3. Gate the module with `#[cfg(feature = "platform-<name>")]` and re-export
///    it from `platform/mod.rs` under the same condition.
/// Uses raw-window-handle 0.6's `HasWindowHandle` + `HasDisplayHandle` traits
/// so wgpu can create a surface without knowing the concrete window type.
pub trait Platform: HasWindowHandle + HasDisplayHandle {
    /// Create the window and return a ready-to-use platform instance.
    ///
    /// `wgpu` will call `window_handle()` / `display_handle()` (rwh 0.6)
    /// to create the surface; both must be valid after this returns.
    fn create_window(title: &str, width: u32, height: u32) -> Result<Self, PlatformError>
    where
        Self: Sized;

    /// Drain the OS event queue and return engine-native events.
    ///
    /// Must be called once per frame **before** updating the world.
    /// Returns an empty vec when there are no pending events.
    fn poll_events(&mut self) -> Vec<PlatformEvent>;

    /// Current inner (drawable) size in physical pixels.
    fn inner_size(&self) -> (u32, u32);

    /// Ask the OS to redraw as soon as possible.
    fn request_redraw(&self);
}
