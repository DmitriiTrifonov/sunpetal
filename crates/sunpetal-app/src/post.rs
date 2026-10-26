//! The passes after the screens are drawn into a window-sized picture: bloom (a bright pass, then a
//! blur at half size, added back) and FXAA. With both off nothing here runs and the screens go
//! straight to the window.
use bytemuck::{Pod, Zeroable};

/// Light below this is not bloomed.
const THRESHOLD: f32 = 0.6;

const SHADER: &str = r#"
struct P {
    a: vec4<f32>,   // the source's texel size, the blur's direction
    b: vec4<f32>,   // bloom strength, threshold, fxaa
};
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var bloom: texture_2d<f32>;

struct Out { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    let c = vec2<f32>(f32(i & 1u), f32((i >> 1u) & 1u));
    var o: Out;
    o.pos = vec4<f32>(c.x * 2.0 - 1.0, 1.0 - c.y * 2.0, 0.0, 1.0);
    o.uv = c;
    return o;
}

fn s(t: texture_2d<f32>, uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(t, samp, uv, 0.0).rgb;
}

// the picture at half size, only what is brighter than the threshold (four taps: a box)
@fragment
fn fs_bright(in: Out) -> @location(0) vec4<f32> {
    let t = p.a.xy;
    let c = 0.25 * (s(tex, in.uv + t * vec2<f32>(-0.5, -0.5)) + s(tex, in.uv + t * vec2<f32>(0.5, -0.5))
        + s(tex, in.uv + t * vec2<f32>(-0.5, 0.5)) + s(tex, in.uv + t * vec2<f32>(0.5, 0.5)));
    let l = max(c.r, max(c.g, c.b));
    let k = max(l - p.b.y, 0.0) / max(l, 0.0001);
    return vec4<f32>(c * k, 1.0);
}

@fragment
fn fs_blur(in: Out) -> @location(0) vec4<f32> {
    let d = p.a.zw * p.a.xy * 1.5;
    var c = s(tex, in.uv) * 0.227027;
    c = c + (s(tex, in.uv + d) + s(tex, in.uv - d)) * 0.1945946;
    c = c + (s(tex, in.uv + d * 2.0) + s(tex, in.uv - d * 2.0)) * 0.1216216;
    c = c + (s(tex, in.uv + d * 3.0) + s(tex, in.uv - d * 3.0)) * 0.054054;
    c = c + (s(tex, in.uv + d * 4.0) + s(tex, in.uv - d * 4.0)) * 0.016216;
    return vec4<f32>(c, 1.0);
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.299, 0.587, 0.114));
}

// FXAA (Lottes' first version): a blur along the edge's direction where the picture has an edge
fn fxaa(uv: vec2<f32>, t: vec2<f32>) -> vec3<f32> {
    let m = s(tex, uv);
    let nw = s(tex, uv + vec2<f32>(-1.0, -1.0) * t);
    let ne = s(tex, uv + vec2<f32>(1.0, -1.0) * t);
    let sw = s(tex, uv + vec2<f32>(-1.0, 1.0) * t);
    let se = s(tex, uv + vec2<f32>(1.0, 1.0) * t);
    let lm = luma(m);
    let lnw = luma(nw);
    let lne = luma(ne);
    let lsw = luma(sw);
    let lse = luma(se);
    let lo = min(lm, min(min(lnw, lne), min(lsw, lse)));
    let hi = max(lm, max(max(lnw, lne), max(lsw, lse)));
    if (hi - lo < max(0.03, hi * 0.125)) {
        return m;
    }
    var dir = vec2<f32>(-((lnw + lne) - (lsw + lse)), (lnw + lsw) - (lne + lse));
    let reduce = max((lnw + lne + lsw + lse) * 0.03125, 0.0078125);
    let rcp = 1.0 / (min(abs(dir.x), abs(dir.y)) + reduce);
    dir = clamp(dir * rcp, vec2<f32>(-8.0), vec2<f32>(8.0)) * t;
    let a = 0.5 * (s(tex, uv + dir * (1.0 / 3.0 - 0.5)) + s(tex, uv + dir * (2.0 / 3.0 - 0.5)));
    let b = a * 0.5 + 0.25 * (s(tex, uv + dir * -0.5) + s(tex, uv + dir * 0.5));
    let lb = luma(b);
    if (lb < lo || lb > hi) {
        return a;
    }
    return b;
}

@fragment
fn fs_comp(in: Out) -> @location(0) vec4<f32> {
    var c = s(tex, in.uv);
    if (p.b.z > 0.5) {
        c = fxaa(in.uv, p.a.xy);
    }
    if (p.b.x > 0.0) {
        c = c + s(bloom, in.uv) * p.b.x;
    }
    return vec4<f32>(c, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    a: [f32; 4],
    b: [f32; 4],
}

/// What does not depend on the window's size: the pipelines.
pub struct Kit {
    bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bright: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    comp: wgpu::RenderPipeline,
}

struct Pass {
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

/// The pictures the passes go through, for one window size.
pub struct Targets {
    pub size: (u32, u32),
    pub scene: wgpu::TextureView,
    half: (wgpu::TextureView, wgpu::TextureView),
    /// bright, blur across, blur down, composite
    passes: [Pass; 4],
}

impl Kit {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Kit {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                tex(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex(3),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let make = |entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Kit { bright: make("fs_bright"), blur: make("fs_blur"), comp: make("fs_comp"), bgl, sampler }
    }
}

impl Targets {
    pub fn new(device: &wgpu::Device, kit: &Kit, format: wgpu::TextureFormat, w: u32, h: u32) -> Targets {
        let (w, h) = (w.max(1), h.max(1));
        let (hw, hh) = ((w / 2).max(1), (h / 2).max(1));
        let view = |w: u32, h: u32, label: &str| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let scene = view(w, h, "scene");
        let (a, b) = (view(hw, hh, "bloom a"), view(hw, hh, "bloom b"));
        let pass = |src: &wgpu::TextureView, extra: &wgpu::TextureView| {
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: std::mem::size_of::<Params>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &kit.bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&kit.sampler) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(extra) },
                ],
            });
            Pass { uniform, bind }
        };
        let passes = [pass(&scene, &scene), pass(&a, &a), pass(&b, &b), pass(&scene, &a)];
        Targets { size: (w, h), scene, half: (a, b), passes }
    }

    /// Runs the passes over the scene into `surface`.
    pub fn run(&self, kit: &Kit, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, surface: &wgpu::TextureView, bloom: f32, fxaa: bool) {
        let (w, h) = (self.size.0 as f32, self.size.1 as f32);
        let (hw, hh) = ((self.size.0 / 2).max(1) as f32, (self.size.1 / 2).max(1) as f32);
        let write = |p: &Pass, a: [f32; 4], b: [f32; 4]| queue.write_buffer(&p.uniform, 0, bytemuck::bytes_of(&Params { a, b }));
        write(&self.passes[0], [1.0 / w, 1.0 / h, 0.0, 0.0], [0.0, THRESHOLD, 0.0, 0.0]);
        write(&self.passes[1], [1.0 / hw, 1.0 / hh, 1.0, 0.0], [0.0; 4]);
        write(&self.passes[2], [1.0 / hw, 1.0 / hh, 0.0, 1.0], [0.0; 4]);
        write(&self.passes[3], [1.0 / w, 1.0 / h, 0.0, 0.0], [bloom * 0.8, 0.0, if fxaa { 1.0 } else { 0.0 }, 0.0]);
        let go = |enc: &mut wgpu::CommandEncoder, pipeline: &wgpu::RenderPipeline, pass: &Pass, to: &wgpu::TextureView| {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: to,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(pipeline);
            rp.set_bind_group(0, &pass.bind, &[]);
            rp.draw(0..4, 0..1);
        };
        if bloom > 0.0 {
            go(enc, &kit.bright, &self.passes[0], &self.half.0);
            go(enc, &kit.blur, &self.passes[1], &self.half.1);
            go(enc, &kit.blur, &self.passes[2], &self.half.0);
        }
        go(enc, &kit.comp, &self.passes[3], surface);
    }
}
