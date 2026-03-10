//! wgpu render context — device, queue, surface, and frame lifecycle.

use wgpu::{
    Adapter, Backends, Device, DeviceDescriptor, Features, Instance, InstanceDescriptor, Limits,
    MemoryHints, PowerPreference, Queue, RequestAdapterOptions, Surface, SurfaceConfiguration,
    SurfaceTexture, TextureFormat, TextureUsages, TextureView, TextureViewDescriptor,
};
use winit::window::Window;

/// Encapsulates the wgpu instance, adapter, device, queue, and surface.
///
/// Created once per window. Call `resize` whenever the window dimensions change.
pub struct RenderContext {
    pub instance: Instance,
    pub adapter: Adapter,
    pub device: Device,
    pub queue: Queue,
    pub surface: Surface<'static>,
    pub surface_config: SurfaceConfiguration,
    pub surface_format: TextureFormat,
}

impl RenderContext {
    /// Create a `RenderContext` from a `winit::window::Window`.
    ///
    /// Blocks the calling thread via `pollster::block_on`.
    ///
    /// # Safety
    /// The window must live at least as long as the returned `RenderContext`.
    pub fn new(
        window: &winit::window::Window,
        width: u32,
        height: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let instance = Instance::new(&InstanceDescriptor {
            backends: Backends::all(),
            ..Default::default()
        });

        // wgpu 25: create_surface_unsafe returns Surface<'static>.
        // SAFETY: window outlives the surface — both are owned by the demo struct.
        let surface: Surface<'static> = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::from_window(window)?)?
        };

        let adapter = pollster::block_on(instance.request_adapter(&RequestAdapterOptions {
            power_preference: PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .map_err(|e| format!("no suitable wgpu adapter: {e}"))?;

        // wgpu 25: request_device takes only DeviceDescriptor (no trace path argument).
        let (device, queue) = pollster::block_on(adapter.request_device(&DeviceDescriptor {
            label: Some("forge_ecs device"),
            required_features: Features::empty(),
            required_limits: Limits::default(),
            memory_hints: MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let surface_config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            surface,
            surface_config,
            surface_format,
        })
    }

    /// Reconfigure the surface after a window resize.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
    }

    /// Ensure the surface configuration matches the window's current physical size.
    ///
    /// Some platforms deliver redraw before resize during fullscreen / DPI changes.
    /// Syncing here prevents rendering against stale surface dimensions.
    pub fn sync_with_window(&mut self, window: &Window) {
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        if size.width != self.surface_config.width || size.height != self.surface_config.height {
            self.resize(size.width, size.height);
        }
    }

    /// Acquire the next frame texture and create a view for rendering.
    ///
    /// Returns `None` when the surface is lost (e.g. window minimised on some platforms).
    pub fn begin_frame(&self) -> Option<(SurfaceTexture, TextureView)> {
        let texture = self.surface.get_current_texture().ok()?;
        let view = texture
            .texture
            .create_view(&TextureViewDescriptor::default());
        Some((texture, view))
    }

    /// Present the frame to the screen.
    pub fn end_frame(&self, texture: SurfaceTexture) {
        texture.present();
    }
}
