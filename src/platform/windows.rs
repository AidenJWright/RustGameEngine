//! Windows platform implementation using `winit 0.30`.
//!
//! Maps all `winit::event::Event` variants to engine-native `PlatformEvent`.
//! Every mapping is commented so a second implementor can follow the same pattern.

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use winit::{
    event::{ElementState, MouseButton as WinitMouseButton, WindowEvent},
    event_loop::EventLoop,
    keyboard::{KeyCode as WinitKeyCode, PhysicalKey},
    window::{Window, WindowAttributes},
};

use super::traits::{KeyCode, MouseButton, Platform, PlatformError, PlatformEvent};

/// `winit 0.30`-backed platform — owns the `EventLoop` and `Window`.
pub struct WinitPlatform {
    /// The actual OS window.
    pub window: Window,
    /// Buffered events collected during the last `poll_events` call.
    buffered_events: Vec<PlatformEvent>,
    /// The event loop, held here until the caller takes it for `run`.
    event_loop: Option<EventLoop<()>>,
}

impl WinitPlatform {
    /// Direct access to the underlying `winit::window::Window`.
    ///
    /// Needed by `imgui-winit-support` which takes a `&winit::window::Window`.
    pub fn window(&self) -> &Window {
        &self.window
    }

    /// Take ownership of the `EventLoop` (can only be called once).
    pub fn take_event_loop(&mut self) -> Option<EventLoop<()>> {
        self.event_loop.take()
    }
}

impl Platform for WinitPlatform {
    fn create_window(title: &str, width: u32, height: u32) -> Result<Self, PlatformError> {
        // In winit 0.30, EventLoop::new() returns a Result.
        let event_loop = EventLoop::new()
            .map_err(|e| PlatformError::EventLoop(e.to_string()))?;

        // In winit 0.30, windows are created via the ActiveEventLoop inside the
        // event loop callback. For pre-loop creation we use create_window directly
        // on the event_loop using the OwnedDisplayHandle approach.
        let attrs = WindowAttributes::default()
            .with_title(title)
            .with_inner_size(winit::dpi::PhysicalSize::new(width, height))
            .with_resizable(true);

        // SAFETY: creating before loop.run is safe on Windows/macOS/Linux.
        #[allow(deprecated)]
        let window = event_loop
            .create_window(attrs)
            .map_err(|e| PlatformError::WindowCreation(e.to_string()))?;

        Ok(Self {
            window,
            buffered_events: Vec::new(),
            event_loop: Some(event_loop),
        })
    }

    fn poll_events(&mut self) -> Vec<PlatformEvent> {
        std::mem::take(&mut self.buffered_events)
    }

    fn inner_size(&self) -> (u32, u32) {
        let s = self.window.inner_size();
        (s.width, s.height)
    }

    fn request_redraw(&self) {
        self.window.request_redraw();
    }
}

// ---------------------------------------------------------------------------
// raw-window-handle 0.6 integration (required by wgpu 25 surface creation)
// ---------------------------------------------------------------------------

impl HasWindowHandle for WinitPlatform {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.window.window_handle()
    }
}

impl HasDisplayHandle for WinitPlatform {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.window.display_handle()
    }
}

// ---------------------------------------------------------------------------
// Event mapping helpers (used by demo's event loop)
// ---------------------------------------------------------------------------

/// Convert a `winit 0.30` `PhysicalKey` to engine `KeyCode`.
///
/// A second platform maps its own key enum here using the same pattern.
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

/// Convert a `winit 0.30` `MouseButton` to engine `MouseButton`.
pub fn map_mouse_button(b: WinitMouseButton) -> MouseButton {
    match b {
        // Left / right / middle are the three standard buttons.
        WinitMouseButton::Left => MouseButton::Left,
        WinitMouseButton::Right => MouseButton::Right,
        WinitMouseButton::Middle => MouseButton::Middle,
        // winit 0.30 added Back/Forward — map to Other so the engine ignores them.
        WinitMouseButton::Back => MouseButton::Other(3),
        WinitMouseButton::Forward => MouseButton::Other(4),
        WinitMouseButton::Other(n) => MouseButton::Other(n),
    }
}

/// Convert a `winit 0.30` `WindowEvent` to zero or one `PlatformEvent`.
///
/// Returns `None` for events the engine doesn't care about (e.g. focus changes).
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
