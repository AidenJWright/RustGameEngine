//! Draw command queue — accumulates commands per frame, flushes sorted by pipeline.

use wgpu::{
    CommandEncoder, LoadOp, Operations, RenderPassColorAttachment, RenderPassDescriptor, StoreOp,
    TextureView,
};

use super::context::RenderContext;
use super::pipeline::{CirclePipeline, RectPipeline, Uniforms};

// ---------------------------------------------------------------------------
// Draw command enum
// ---------------------------------------------------------------------------

/// A single renderable primitive queued for the current frame.
#[derive(Debug, Clone)]
pub enum DrawCommand {
    /// A filled circle drawn via SDF.
    Circle {
        x: f32,
        y: f32,
        radius: f32,
        color: [f32; 4],
    },
    /// An axis-aligned filled rectangle.
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: [f32; 4],
    },
}

// ---------------------------------------------------------------------------
// Draw queue
// ---------------------------------------------------------------------------

/// Accumulates [`DrawCommand`]s each frame and submits them in one pass.
///
/// Commands are sorted by pipeline (all circles first, then rects) to minimise
/// GPU state-change overhead.
#[derive(Default)]
pub struct DrawQueue {
    commands: Vec<DrawCommand>,
}

impl DrawQueue {
    /// Create an empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a draw command.
    pub fn push(&mut self, cmd: DrawCommand) {
        self.commands.push(cmd);
    }

    /// Flush all queued commands to the GPU, then clear the queue.
    ///
    /// Performs a single render pass with a clear + all geometry.
    /// Circles are drawn first (sorted by enum discriminant), then rects.
    pub fn flush(
        &mut self,
        context: &RenderContext,
        view: &TextureView,
        encoder: &mut CommandEncoder,
        circle_pipeline: &CirclePipeline,
        rect_pipeline: &RectPipeline,
        clear_color: [f64; 4],
    ) {
        let (width, height) = (
            context.surface_config.width as f32,
            context.surface_config.height as f32,
        );

        // Sort: circles first (variant 0), rects second (variant 1).
        self.commands.sort_by_key(|c| match c {
            DrawCommand::Circle { .. } => 0u8,
            DrawCommand::Rect { .. } => 1u8,
        });

        // Begin render pass with a clear.
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("scene pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(wgpu::Color {
                        r: clear_color[0],
                        g: clear_color[1],
                        b: clear_color[2],
                        a: clear_color[3],
                    }),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        // --- Circles ---
        self.commands
            .iter()
            .filter_map(|c| {
                if let DrawCommand::Circle {
                    x,
                    y,
                    radius,
                    color,
                } = c
                {
                    Some((*x, *y, *radius, *color))
                } else {
                    None
                }
            })
            .for_each(|(x, y, radius, color)| {
                let uniforms = Uniforms {
                    position: [x, y],
                    size: [radius * 2.0, radius * 2.0],
                    color,
                    resolution: [width, height],
                    _pad: [0.0; 2],
                };
                let uniform_buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("circle draw uniform"),
                    size: std::mem::size_of::<Uniforms>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                context
                    .queue
                    .write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
                let bind_group = context
                    .device
                    .create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("circle draw bind group"),
                        layout: &circle_pipeline.bind_group_layout,
                        entries: &[wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniform_buffer.as_entire_binding(),
                        }],
                    });

                // Use a per-command bind group/buffer so each draw sees its own
                // transform/size/color pair, avoiding last-write overwrite.
                pass.set_pipeline(&circle_pipeline.pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.draw(0..4, 0..1);
            });

        // --- Rects ---
        self.commands
            .iter()
            .filter_map(|c| {
                if let DrawCommand::Rect {
                    x,
                    y,
                    width: w,
                    height: h,
                    color,
                } = c
                {
                    Some((*x, *y, *w, *h, *color))
                } else {
                    None
                }
            })
            .for_each(|(x, y, w, h, color)| {
                let uniforms = Uniforms {
                    position: [x, y],
                    size: [w, h],
                    color,
                    resolution: [width, height],
                    _pad: [0.0; 2],
                };
                let uniform_buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("rect draw uniform"),
                    size: std::mem::size_of::<Uniforms>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                context
                    .queue
                    .write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
                let bind_group = context
                    .device
                    .create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("rect draw bind group"),
                        layout: &rect_pipeline.bind_group_layout,
                        entries: &[wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniform_buffer.as_entire_binding(),
                        }],
                    });
                pass.set_pipeline(&rect_pipeline.pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.draw(0..4, 0..1);
            });

        drop(pass); // end render pass — required before submitting encoder
        self.commands.clear();
    }
}
