//! Render pipelines for circle, rect, and triangle primitives.
//!
//! Both pipelines share a uniform layout:
//!   - binding 0: `Uniforms` buffer with position, size, color, resolution,
//!     and rotation.

use bytemuck::{Pod, Zeroable};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingType, Buffer, BufferBindingType, BufferDescriptor, BufferUsages,
    ColorTargetState, ColorWrites, Device, FragmentState, MultisampleState,
    PipelineLayoutDescriptor, PrimitiveState, Queue, RenderPipeline, RenderPipelineDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, TextureFormat, VertexState,
};

// ---------------------------------------------------------------------------
// Shared uniform struct (matches WGSL layout)
// ---------------------------------------------------------------------------

/// Per-draw uniforms uploaded before each draw call.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Uniforms {
    /// World-space X, Y centre of the primitive.
    pub position: [f32; 2],
    /// Half-extents or radius×2: `[width, height]` for rect, `[r*2, r*2]` for circle.
    pub size: [f32; 2],
    /// Linear RGBA color.
    pub color: [f32; 4],
    /// Surface dimensions in pixels — needed for NDC conversion inside the shader.
    pub resolution: [f32; 2],
    /// Rotation angle in radians. Used by triangle rendering; ignored by other primitives.
    pub rotation: f32,
    /// Padding to satisfy 16-byte alignment rule.
    pub _pad: f32,
}

// ---------------------------------------------------------------------------
// Circle pipeline — SDF quad
// ---------------------------------------------------------------------------

/// WGSL shader for a fullscreen quad that discards fragments outside the circle.
const CIRCLE_SHADER: &str = r#"
struct Uniforms {
    position: vec2<f32>,
    size:     vec2<f32>,    // size.x = diameter (radius * 2)
    color:    vec4<f32>,
    resolution: vec2<f32>,
    rotation: f32,
    _pad:     f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Emit a quad of 4 vertices covering the circle's bounding box.
// Vertices are generated procedurally from vertex_index (0..3 triangle-strip).
@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOut {
    // Unit quad corners: BL, BR, TL, TR
    let corners = array<vec2<f32>, 4>(
        vec2<f32>(-0.5, -0.5),
        vec2<f32>( 0.5, -0.5),
        vec2<f32>(-0.5,  0.5),
        vec2<f32>( 0.5,  0.5),
    );
    let corner = corners[vi];

    // World-space position of this vertex.
    let world_pos = u.position + corner * u.size;

    // Convert world space (origin = top-left, Y down) to NDC.
    let ndc = vec2<f32>(
        world_pos.x / u.resolution.x * 2.0 - 1.0,
        1.0 - (world_pos.y / u.resolution.y * 2.0),
    );

    var out: VertexOut;
    out.clip_pos = vec4<f32>(ndc.x, ndc.y, 0.0, 1.0);
    out.uv = corner + vec2<f32>(0.5); // map [-0.5,0.5] -> [0,1]
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // SDF circle: discard if farther than 0.5 from centre.
    let d = length(in.uv - vec2<f32>(0.5));
    if d > 0.5 {
        discard;
    }
    return u.color;
}
"#;

/// A render pipeline that draws circles via an SDF fullscreen quad.
pub struct CirclePipeline {
    pub pipeline: RenderPipeline,
    pub bind_group_layout: BindGroupLayout,
    pub uniform_buffer: Buffer,
    pub bind_group: BindGroup,
}

impl CirclePipeline {
    /// Create the pipeline for the given surface format.
    pub fn new(device: &Device, format: TextureFormat) -> Self {
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("circle shader"),
            source: ShaderSource::Wgsl(CIRCLE_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("circle bgl"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX_FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("circle pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("circle pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[], // vertices are procedural
                compilation_options: Default::default(),
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let uniform_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("circle uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("circle bind group"),
            layout: &bind_group_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        Self {
            pipeline,
            bind_group_layout,
            uniform_buffer,
            bind_group,
        }
    }

    /// Upload new uniforms and return the bind group to set before drawing.
    pub fn upload_uniforms(&self, queue: &Queue, uniforms: &Uniforms) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(uniforms));
    }
}

// ---------------------------------------------------------------------------
// Rect pipeline — simple quad
// ---------------------------------------------------------------------------

/// WGSL shader for an axis-aligned filled rectangle.
const RECT_SHADER: &str = r#"
struct Uniforms {
    position: vec2<f32>,
    size:     vec2<f32>,
    color:    vec4<f32>,
    resolution: vec2<f32>,
    rotation: f32,
    _pad:     f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOut {
    let corners = array<vec2<f32>, 4>(
        vec2<f32>(-0.5, -0.5),
        vec2<f32>( 0.5, -0.5),
        vec2<f32>(-0.5,  0.5),
        vec2<f32>( 0.5,  0.5),
    );
    let world_pos = u.position + corners[vi] * u.size;
    // Convert world space (origin = top-left, Y down) to NDC.
    let ndc = vec2<f32>(
        world_pos.x / u.resolution.x * 2.0 - 1.0,
        1.0 - (world_pos.y / u.resolution.y * 2.0),
    );
    var out: VertexOut;
    out.clip_pos = vec4<f32>(ndc.x, ndc.y, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return u.color;
}
"#;

/// A render pipeline that draws axis-aligned rectangles.
pub struct RectPipeline {
    pub pipeline: RenderPipeline,
    pub bind_group_layout: BindGroupLayout,
    pub uniform_buffer: Buffer,
    pub bind_group: BindGroup,
}

impl RectPipeline {
    /// Create the pipeline for the given surface format.
    pub fn new(device: &Device, format: TextureFormat) -> Self {
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("rect shader"),
            source: ShaderSource::Wgsl(RECT_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("rect bgl"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX_FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("rect pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("rect pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let uniform_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("rect uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("rect bind group"),
            layout: &bind_group_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        Self {
            pipeline,
            bind_group_layout,
            uniform_buffer,
            bind_group,
        }
    }

    /// Upload new uniforms before drawing.
    pub fn upload_uniforms(&self, queue: &Queue, uniforms: &Uniforms) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(uniforms));
    }
}

// ---------------------------------------------------------------------------
// Triangle pipeline — rotated SDF quad
// ---------------------------------------------------------------------------

/// WGSL shader for a filled equilateral triangle inside a square bounding box.
const TRIANGLE_SHADER: &str = r#"
struct Uniforms {
    position: vec2<f32>,
    size:     vec2<f32>,
    color:    vec4<f32>,
    resolution: vec2<f32>,
    rotation: f32,
    _pad:     f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOut {
    let corners = array<vec2<f32>, 4>(
        vec2<f32>(-0.5, -0.5),
        vec2<f32>( 0.5, -0.5),
        vec2<f32>(-0.5,  0.5),
        vec2<f32>( 0.5,  0.5),
    );
    let corner = corners[vi];
    let world_pos = u.position + corner * u.size;
    let ndc = vec2<f32>(
        world_pos.x / u.resolution.x * 2.0 - 1.0,
        1.0 - (world_pos.y / u.resolution.y * 2.0),
    );

    var out: VertexOut;
    out.clip_pos = vec4<f32>(ndc.x, ndc.y, 0.0, 1.0);
    out.uv = corner + vec2<f32>(0.5);
    return out;
}

fn tri_sdf(p_in: vec2<f32>) -> f32 {
    let k = sqrt(3.0);
    var q = vec2<f32>(abs(p_in.x) - 0.5, p_in.y + 0.5 / k);
    if q.x + k * q.y > 0.0 {
        q = vec2<f32>(q.x - k * q.y, -k * q.x - q.y) * 0.5;
    }
    q.x -= clamp(q.x, -1.0, 0.0);
    return -length(q) * sign(q.y);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let p = in.uv - vec2<f32>(0.5);
    let c = cos(u.rotation);
    let s = sin(u.rotation);
    let rotated = vec2<f32>(
        p.x * c - p.y * s,
        p.x * s + p.y * c,
    );
    if tri_sdf(rotated) > 0.0 {
        discard;
    }
    return u.color;
}
"#;

/// A render pipeline that draws rotated triangles via an SDF quad.
pub struct TrianglePipeline {
    pub pipeline: RenderPipeline,
    pub bind_group_layout: BindGroupLayout,
    pub uniform_buffer: Buffer,
    pub bind_group: BindGroup,
}

impl TrianglePipeline {
    /// Create the pipeline for the given surface format.
    pub fn new(device: &Device, format: TextureFormat) -> Self {
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("triangle shader"),
            source: ShaderSource::Wgsl(TRIANGLE_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("triangle bgl"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX_FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("triangle pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("triangle pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let uniform_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("triangle uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("triangle bind group"),
            layout: &bind_group_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        Self {
            pipeline,
            bind_group_layout,
            uniform_buffer,
            bind_group,
        }
    }

    /// Upload new uniforms before drawing.
    pub fn upload_uniforms(&self, queue: &Queue, uniforms: &Uniforms) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(uniforms));
    }
}
