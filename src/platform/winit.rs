//! Cross-platform `winit` backend for window creation and event translation.
//!
//! Maps `winit::event::WindowEvent` variants to engine-native `PlatformEvent`.
//! Window creation is handled externally via `ActiveEventLoop::create_window`;
//! this module simply wraps the resulting `Window`.

use winit::{
    event::{ElementState, MouseButton as WinitMouseButton, WindowEvent},
    keyboard::{KeyCode as WinitKeyCode, PhysicalKey},
    window::Window,
};

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

/// Engine-native key code — maps from winit-provided key input.
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

/// Engine-native event emitted by `map_window_event`.
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
// Winit platform
// ---------------------------------------------------------------------------

/// `winit 0.30`-backed platform — wraps an OS `Window`.
///
/// Window creation is delegated to `ActiveEventLoop::create_window` inside
/// an `ApplicationHandler::resumed` implementation; pass the resulting
/// `Window` to [`WinitPlatform::from_window`].
pub struct WinitPlatform {
    /// The actual OS window.
    pub window: Window,
}

impl WinitPlatform {
    /// Wrap an already-created `Window`.
    pub fn from_window(window: Window) -> Self {
        Self { window }
    }

    /// Direct access to the underlying `winit::window::Window`.
    ///
    /// Needed by `imgui-winit-support` which takes a `&winit::window::Window`.
    pub fn window(&self) -> &Window {
        &self.window
    }

    /// Current inner (drawable) size in physical pixels.
    pub fn inner_size(&self) -> (u32, u32) {
        let s = self.window.inner_size();
        (s.width, s.height)
    }

    /// Ask the OS to redraw as soon as possible.
    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }
}

// ---------------------------------------------------------------------------
// Event mapping helpers
// ---------------------------------------------------------------------------

/// Convert a `winit::event::PhysicalKey` to engine `KeyCode`.
pub fn map_physical_key(key: PhysicalKey) -> KeyCode {
    match key {
        PhysicalKey::Code(WinitKeyCode::Escape) => KeyCode::Escape,
        PhysicalKey::Code(WinitKeyCode::Space) => KeyCode::Space,
        PhysicalKey::Code(WinitKeyCode::Enter) => KeyCode::Return,
        PhysicalKey::Code(WinitKeyCode::ArrowLeft) => KeyCode::Left,
        PhysicalKey::Code(WinitKeyCode::ArrowRight) => KeyCode::Right,
        PhysicalKey::Code(WinitKeyCode::ArrowUp) => KeyCode::Up,
        PhysicalKey::Code(WinitKeyCode::ArrowDown) => KeyCode::Down,
        // Anything unrecognised maps to Other(0) — extend as needed.
        _ => KeyCode::Other(0),
    }
}

/// Convert a `winit::event::MouseButton` to engine `MouseButton`.
pub fn map_mouse_button(b: WinitMouseButton) -> MouseButton {
    match b {
        WinitMouseButton::Left => MouseButton::Left,
        WinitMouseButton::Right => MouseButton::Right,
        WinitMouseButton::Middle => MouseButton::Middle,
        // winit 0.30 added Back/Forward — map to Other so the engine ignores them.
        WinitMouseButton::Back => MouseButton::Other(3),
        WinitMouseButton::Forward => MouseButton::Other(4),
        WinitMouseButton::Other(n) => MouseButton::Other(n),
    }
}

/// Convert a `winit::event::WindowEvent` to zero or one `PlatformEvent`.
///
/// Returns `None` for events the engine doesn't care about (for example focus changes).
pub fn map_window_event(event: &WindowEvent) -> Option<PlatformEvent> {
    match event {
        // Window close button or Alt-F4.
        WindowEvent::CloseRequested => Some(PlatformEvent::Quit),

        // Resize — physical pixels.
        WindowEvent::Resized(s) => Some(PlatformEvent::Resized(s.width, s.height)),

        // Redraw request from the OS compositor.
        WindowEvent::RedrawRequested => Some(PlatformEvent::RedrawRequested),

        // Keyboard input — winit 0.30 uses KeyEvent with PhysicalKey.
        WindowEvent::KeyboardInput { event, .. } => {
            let key = map_physical_key(event.physical_key);
            Some(match event.state {
                ElementState::Pressed => PlatformEvent::KeyPressed(key),
                ElementState::Released => PlatformEvent::KeyReleased(key),
            })
        }

        // Cursor position.
        WindowEvent::CursorMoved { position, .. } => {
            Some(PlatformEvent::MouseMoved { x: position.x, y: position.y })
        }

        // Mouse buttons.
        WindowEvent::MouseInput { button, state, .. } => Some(PlatformEvent::MouseButton {
            button: map_mouse_button(*button),
            pressed: *state == ElementState::Pressed,
        }),

        _ => None,
    }
}
