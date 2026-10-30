//! The 3D rendering engine on the GPU (wgpu): a second backend behind the same job stream the
//! software renderer (`render3d`) draws, this one made for speed at large scales. Этап 4 of
//! `docs/GPU-BACKEND.md`: the whole picture except the wireframe polygons — the exact rasterizer
//! rules of the software engine (fan triangulation, the top-left ownership of edges,
//! perspective-correct colour and texture coordinates, z/w depth, the less-than depth test, the
//! translucent polygons in a second pass in the job's order with the DS blend
//! `(src*(a+1)+dst*(31-a))/32` and the shadow masks/shadows, the fog over the finished picture,
//! the rear plane's bitmap as the base) in WGSL, the picture read back as the same words. The
//! textures a frame uses are decoded on the CPU (by the same code the texture packs dump with) into
//! a picture atlas (`Rgba8Uint`), and the shader samples it exactly as the software engine does:
//! nearest, or the alpha-weighted bilinear (a transparent texel's colour never bleeds into its
//! neighbours), wrapped by the texture's flags, blended by the polygon's mode
//! (modulate/decal/toon-highlight). The fog is a second pass over the finished picture (the
//! pixel's depth and fog flag ride in two extra attachments); the rear plane is a full-screen quad
//! over the clear that takes its colour, depth and fog from a 256x256 bitmap. The parity harness
//! (`gpu_parity_check`) scores it against the software renderer on the same filtered job
//! (`stage4_job`).
//!
//! The rules the software engine implements that the GPU one still does not: wireframe polygons,
//! edge marking, the widened view, the outline and the shading, MSAA. `stage4_job` takes the
//! wireframe polygons out of a job, so `render` need not decide per polygon; on a full job it
//! would miss pixels, which stage 5+ will fix.

use super::render3d::{self, GxVertex, Image, Job, Mem, TexKey};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use wgpu::util::DeviceExt;

/// What a frame makes and uploads, counted under `SUNPETAL_GPU_TIMING` (этап 9.0.2):
/// textures, their bytes, buffers, their bytes, bind groups, bytes written by the queue, attachments taken from the pool, atlases kept from the last frame.
static MADE: [std::sync::atomic::AtomicU64; 8] = [const { std::sync::atomic::AtomicU64::new(0) }; 8];

fn made(i: usize, n: u64) {
    MADE[i].fetch_add(n, std::sync::atomic::Ordering::Relaxed);
}

fn made_texture(device: &wgpu::Device, d: &wgpu::TextureDescriptor) -> wgpu::Texture {
    let px = d.format.block_copy_size(None).unwrap_or(4) as u64;
    made(0, 1);
    made(1, d.size.width as u64 * d.size.height as u64 * d.size.depth_or_array_layers as u64 * px);
    device.create_texture(d)
}

fn made_buffer(device: &wgpu::Device, d: &wgpu::BufferDescriptor) -> wgpu::Buffer {
    made(2, 1);
    made(3, d.size);
    device.create_buffer(d)
}

fn made_buffer_init(device: &wgpu::Device, d: &wgpu::util::BufferInitDescriptor) -> wgpu::Buffer {
    made(2, 1);
    made(3, d.contents.len() as u64);
    made(5, d.contents.len() as u64);
    device.create_buffer_init(d)
}

fn made_bind_group(device: &wgpu::Device, d: &wgpu::BindGroupDescriptor) -> wgpu::BindGroup {
    made(4, 1);
    device.create_bind_group(d)
}

/// The counters, read and zeroed.
fn take_made() -> [u64; 8] {
    std::array::from_fn(|i| MADE[i].swap(0, std::sync::atomic::Ordering::Relaxed))
}

/// A wgpu device of the process, made on first use and kept (the parity harness renders many
/// frames; the game loop will too).
struct Ctx {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    rear_pipeline: wgpu::RenderPipeline,
    bgl_rear: wgpu::BindGroupLayout,
    fog_pipeline: wgpu::RenderPipeline,
    bgl_fog: wgpu::BindGroupLayout,
    outline_pipeline: wgpu::RenderPipeline,
    bgl_outline: wgpu::BindGroupLayout,
    shade_pipeline: wgpu::RenderPipeline,
    bgl_shade: wgpu::BindGroupLayout,
    rows_pipeline: wgpu::RenderPipeline,
    bgl_rows: wgpu::BindGroupLayout,
    skyrow_pipeline: wgpu::RenderPipeline,
    bgl_skyrow: wgpu::BindGroupLayout,
    extend_pipeline: wgpu::RenderPipeline,
    bgl_extend: wgpu::BindGroupLayout,
    blend_pipeline: wgpu::RenderPipeline,
    bgl_blend: wgpu::BindGroupLayout,
    mask_pipeline: wgpu::RenderPipeline,
    resolve_pipeline: wgpu::RenderPipeline,
    bgl_resolve: wgpu::BindGroupLayout,
    bgl_mask: wgpu::BindGroupLayout,
    tiled_pipeline: wgpu::RenderPipeline,
    bgl_tiled: wgpu::BindGroupLayout,
    /// The attachments the frames are done with, to be taken again (этап 9.1.1).
    pool: Mutex<Vec<(PoolKey, wgpu::Texture)>>,
    /// The small per-frame buffers (uniforms, the full-screen triangle, the resolve factor) the same way (этап 9.1.4).
    buf_pool: Mutex<Vec<(BufKey, wgpu::Buffer)>>,
    /// The last frame's texture atlas (этап 9.1.6).
    atlas_cache: Mutex<Option<Arc<AtlasEntry>>>,
}

/// The atlas of a frame's textures with where each picture sits in it; `_pics` keep the pictures alive, so
/// that the addresses in `key` are never another picture's.
struct AtlasEntry {
    key: Vec<(u32, u32, usize, u32, u32, u32)>,
    tex: wgpu::Texture,
    regions: HashMap<(u32, u32), TexInfo>,
    _pics: Vec<Arc<Image>>,
}

static CTX: OnceLock<Option<Ctx>> = OnceLock::new();

fn ctx() -> Result<&'static Ctx, String> {
    CTX.get_or_init(|| make_ctx().ok())
        .as_ref()
        .ok_or_else(|| "no GPU device for the 3D renderer".to_string())
}

fn make_ctx() -> Result<Ctx, String> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        .map_err(|_| "no GPU adapter")?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        .map_err(|e| e.to_string())?;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("render3d_gpu"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let uniform = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let storage = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let tex = |binding: u32, sample: wgpu::TextureSampleType| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: sample,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    // the geometry pass: uniforms, the triangles, the texture atlas
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("render3d_gpu"),
        entries: &[uniform(0), storage(1), tex(2, wgpu::TextureSampleType::Uint)],
    });
    // the rear plane's bitmap: uniforms and its 256x256 texture
    let bgl_rear = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("render3d_gpu rear"),
        entries: &[uniform(0), tex(3, wgpu::TextureSampleType::Uint)],
    });
    // the fog pass: uniforms, the finished picture's word, fog flag, depth, the outline level, the
    // shade level and the polygon ID (the edge marking reads it)
    let bgl_fog = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("render3d_gpu fog"),
        entries: &[
            uniform(0),
            tex(4, wgpu::TextureSampleType::Uint),
            tex(5, wgpu::TextureSampleType::Uint),
            tex(6, wgpu::TextureSampleType::Float { filterable: false }),
            tex(8, wgpu::TextureSampleType::Uint),
            tex(14, wgpu::TextureSampleType::Uint),
            tex(15, wgpu::TextureSampleType::Uint),
        ],
    });
    // the outline pass (этап 7.1): uniforms and the finished picture's word, depth, ID, cover, trans
    let bgl_outline = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("render3d_gpu outline"),
        entries: &[
            uniform(0),
            tex(4, wgpu::TextureSampleType::Uint),
            tex(6, wgpu::TextureSampleType::Float { filterable: false }),
            tex(11, wgpu::TextureSampleType::Uint),
            tex(12, wgpu::TextureSampleType::Uint),
            tex(13, wgpu::TextureSampleType::Uint),
        ],
    });
    // the shade pass (этап 7.2): uniforms and the finished picture's word, depth, cover, trans
    let bgl_shade = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("render3d_gpu shade"),
        entries: &[
            uniform(0),
            tex(4, wgpu::TextureSampleType::Uint),
            tex(6, wgpu::TextureSampleType::Float { filterable: false }),
            tex(12, wgpu::TextureSampleType::Uint),
            tex(13, wgpu::TextureSampleType::Uint),
        ],
    });
    // the widened view's row sources (этап 7.4): uniforms and the finished depth
    let bgl_rows = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("render3d_gpu rows"),
        entries: &[uniform(0), tex(6, wgpu::TextureSampleType::Float { filterable: false })],
    });
    // the nearest sky row (этап 7.4): uniforms and the row sources
    let bgl_skyrow = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("render3d_gpu sky row"),
        entries: &[uniform(0), tex(16, wgpu::TextureSampleType::Uint)],
    });
    // the widened view's fill (этап 7.4): uniforms and every field of the accumulated picture
    // (the fog flag through a copy — it is not ping-ponged), plus the row sources
    let bgl_extend = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("render3d_gpu wide"),
        entries: &[
            uniform(0),
            tex(4, wgpu::TextureSampleType::Uint),
            tex(5, wgpu::TextureSampleType::Uint),
            tex(6, wgpu::TextureSampleType::Float { filterable: false }),
            tex(8, wgpu::TextureSampleType::Uint),
            tex(11, wgpu::TextureSampleType::Uint),
            tex(12, wgpu::TextureSampleType::Uint),
            tex(13, wgpu::TextureSampleType::Uint),
            tex(16, wgpu::TextureSampleType::Uint),
        ],
    });
    // the translucent pass: uniforms, the triangles, the atlas and the accumulated picture
    let bgl_blend = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("render3d_gpu blend"),
        entries: &[
            uniform(0),
            storage(1),
            tex(2, wgpu::TextureSampleType::Uint),
            tex(7, wgpu::TextureSampleType::Uint),
            tex(8, wgpu::TextureSampleType::Uint),
            tex(9, wgpu::TextureSampleType::Uint),
            tex(10, wgpu::TextureSampleType::Float { filterable: false }),
            tex(11, wgpu::TextureSampleType::Uint),
            tex(12, wgpu::TextureSampleType::Uint),
            tex(13, wgpu::TextureSampleType::Uint),
        ],
    });
    // the shadow mask: uniforms, the triangles and the accumulated depth
    let bgl_mask = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("render3d_gpu mask"),
        entries: &[uniform(0), storage(1), tex(10, wgpu::TextureSampleType::Float { filterable: false })],
    });
    let vbuf = wgpu::VertexBufferLayout {
        array_stride: 8,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 0,
            shader_location: 0,
        }],
    };
    let color_targets: [Option<wgpu::ColorTargetState>; 8] = [
        Some(wgpu::TextureFormat::R32Uint.into()),  // 0 word
        Some(wgpu::TextureFormat::R32Uint.into()),  // 1 fog
        Some(wgpu::TextureFormat::R32Float.into()), // 2 depth
        Some(wgpu::TextureFormat::R32Uint.into()),  // 3 id
        Some(wgpu::TextureFormat::R32Uint.into()),  // 4 stencil
        Some(wgpu::TextureFormat::R32Uint.into()),  // 5 oid
        Some(wgpu::TextureFormat::R32Uint.into()),  // 6 cover
        Some(wgpu::TextureFormat::R32Uint.into()),  // 7 trans
    ];
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("render3d_gpu"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("render3d_gpu"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[Some(vbuf.clone())],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &color_targets,
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    let layout_rear = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("render3d_gpu rear"),
        bind_group_layouts: &[Some(&bgl_rear)],
        immediate_size: 0,
    });
    let rear_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("render3d_gpu rear"),
        layout: Some(&layout_rear),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_full"),
            compilation_options: Default::default(),
            buffers: &[Some(vbuf.clone())],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_rear"),
            compilation_options: Default::default(),
            targets: &color_targets,
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    let layout_fog = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("render3d_gpu fog"),
        bind_group_layouts: &[Some(&bgl_fog)],
        immediate_size: 0,
    });
    let fog_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("render3d_gpu fog"),
        layout: Some(&layout_fog),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_full"),
            compilation_options: Default::default(),
            buffers: &[Some(vbuf.clone())],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_fog"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::TextureFormat::R32Uint.into())],
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    // the outline (этап 7.1): a full-screen pass computing the line level (0-15) per pixel, one
    // R32Uint target the fog pass packs into the word's bits 24-27
    let layout_outline = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("render3d_gpu outline"),
        bind_group_layouts: &[Some(&bgl_outline)],
        immediate_size: 0,
    });
    let outline_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("render3d_gpu outline"),
        layout: Some(&layout_outline),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_full"),
            compilation_options: Default::default(),
            buffers: &[Some(vbuf.clone())],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_outline"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::TextureFormat::R32Uint.into())],
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    // the shading (этап 7.2): a full-screen pass computing the shade level (0-15) per pixel, one
    // R32Uint target the fog pass packs into the word's bits 28-31
    let layout_shade = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("render3d_gpu shade"),
        bind_group_layouts: &[Some(&bgl_shade)],
        immediate_size: 0,
    });
    let shade_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("render3d_gpu shade"),
        layout: Some(&layout_shade),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_full"),
            compilation_options: Default::default(),
            buffers: &[Some(vbuf.clone())],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_shade"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::TextureFormat::R32Uint.into())],
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    // the widened view (этап 7.4), three passes: the source column per row and side (a 2-texel-wide
    // picture), the nearest sky row, then the fill over the whole picture
    let layout_rows = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("render3d_gpu rows"),
        bind_group_layouts: &[Some(&bgl_rows)],
        immediate_size: 0,
    });
    let rows_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("render3d_gpu rows"),
        layout: Some(&layout_rows),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_rows"),
            compilation_options: Default::default(),
            buffers: &[Some(vbuf.clone())],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_rows"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::TextureFormat::Rgba32Uint.into())],
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    let layout_skyrow = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("render3d_gpu sky row"),
        bind_group_layouts: &[Some(&bgl_skyrow)],
        immediate_size: 0,
    });
    let skyrow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("render3d_gpu sky row"),
        layout: Some(&layout_skyrow),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_rows"),
            compilation_options: Default::default(),
            buffers: &[Some(vbuf.clone())],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_skyrow"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::TextureFormat::Rgba32Uint.into())],
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    // the fill's targets: everything the passes after read, except the shadow mask (the engine
    // leaves it alone)
    let extend_targets: [Option<wgpu::ColorTargetState>; 7] = [
        Some(wgpu::TextureFormat::R32Uint.into()),  // 0 word
        Some(wgpu::TextureFormat::R32Uint.into()),  // 1 fog
        Some(wgpu::TextureFormat::R32Float.into()), // 2 depth
        Some(wgpu::TextureFormat::R32Uint.into()),  // 3 id
        Some(wgpu::TextureFormat::R32Uint.into()),  // 4 oid
        Some(wgpu::TextureFormat::R32Uint.into()),  // 5 cover
        Some(wgpu::TextureFormat::R32Uint.into()),  // 6 trans
    ];
    let layout_extend = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("render3d_gpu wide"),
        bind_group_layouts: &[Some(&bgl_extend)],
        immediate_size: 0,
    });
    let extend_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("render3d_gpu wide"),
        layout: Some(&layout_extend),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_full"),
            compilation_options: Default::default(),
            buffers: &[Some(vbuf.clone())],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_extend"),
            compilation_options: Default::default(),
            targets: &extend_targets,
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    let layout_blend = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("render3d_gpu blend"),
        bind_group_layouts: &[Some(&bgl_blend)],
        immediate_size: 0,
    });
    let blend_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("render3d_gpu blend"),
        layout: Some(&layout_blend),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[Some(vbuf.clone())],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_trans"),
            compilation_options: Default::default(),
            targets: &color_targets,
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    // the tiled translucent pass (этап 9.4): one full-screen pass over every translucent and shadow polygon
    let bgl_tiled = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("render3d_gpu tiled"),
        entries: &[
            uniform(0),
            storage(1),
            tex(2, wgpu::TextureSampleType::Uint),
            tex(7, wgpu::TextureSampleType::Uint),
            tex(8, wgpu::TextureSampleType::Uint),
            tex(9, wgpu::TextureSampleType::Uint),
            tex(10, wgpu::TextureSampleType::Float { filterable: false }),
            tex(11, wgpu::TextureSampleType::Uint),
            tex(12, wgpu::TextureSampleType::Uint),
            tex(13, wgpu::TextureSampleType::Uint),
            storage(17),
            storage(18),
            storage(19),
            tex(20, wgpu::TextureSampleType::Uint),
        ],
    });
    let layout_tiled = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("render3d_gpu tiled"),
        bind_group_layouts: &[Some(&bgl_tiled)],
        immediate_size: 0,
    });
    let tiled_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("render3d_gpu tiled"),
        layout: Some(&layout_tiled),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_full"),
            compilation_options: Default::default(),
            buffers: &[Some(vbuf.clone())],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_tiled"),
            compilation_options: Default::default(),
            targets: &color_targets,
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    let layout_mask = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("render3d_gpu mask"),
        bind_group_layouts: &[Some(&bgl_mask)],
        immediate_size: 0,
    });
    let mask_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("render3d_gpu mask"),
        layout: Some(&layout_mask),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[Some(vbuf.clone())],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_mask"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::TextureFormat::R32Uint.into())],
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    // the supersampling resolve (этап 6.2.1): the k x k samples of a pixel averaged into one,
    // so only the final picture is read back; k comes in a uniform
    let bgl_resolve = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("render3d_gpu resolve"),
        entries: &[uniform(0), tex(4, wgpu::TextureSampleType::Uint), uniform(16)],
    });
    let layout_resolve = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("render3d_gpu resolve"),
        bind_group_layouts: &[Some(&bgl_resolve)],
        immediate_size: 0,
    });
    let resolve_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("render3d_gpu resolve"),
        layout: Some(&layout_resolve),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_full"),
            compilation_options: Default::default(),
            buffers: &[Some(vbuf.clone())],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_resolve"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::TextureFormat::R32Uint.into())],
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    Ok(Ctx { device, queue, pipeline, bgl, rear_pipeline, bgl_rear, fog_pipeline, bgl_fog, outline_pipeline, bgl_outline, shade_pipeline, bgl_shade, rows_pipeline, bgl_rows, skyrow_pipeline, bgl_skyrow, extend_pipeline, bgl_extend, blend_pipeline, bgl_blend, mask_pipeline, bgl_mask, resolve_pipeline, bgl_resolve, tiled_pipeline, bgl_tiled, pool: Mutex::new(Vec::new()), buf_pool: Mutex::new(Vec::new()), atlas_cache: Mutex::new(None) })
}

/// Where a texture's picture sits in the frame's atlas, and how its coordinates map onto it.
#[derive(Clone, Copy)]
struct TexInfo {
    /// The picture's top-left corner in the atlas, in texels.
    ux: u32,
    uy: u32,
    /// The picture's size in texels: the texture's own, or its replacement's.
    w: u32,
    h: u32,
    /// The original texture's size in texels, 0 for the DS's own texture (whose coordinates
    /// already are texels). A replacement's coordinates divide by it and multiply by `w`/`h`,
    /// exactly as the software engine's `sample_image` does.
    ow: f32,
    oh: f32,
    /// bit 0: S repeats, bit 1: T repeats, bit 2: S flips, bit 3: T flips, bit 4: a replacement
    /// picture (its coordinates scale).
    flags: u32,
}

const F_REPEAT_S: u32 = 1;
const F_REPEAT_T: u32 = 2;
const F_FLIP_S: u32 = 4;
const F_FLIP_T: u32 = 8;
const F_SCALED: u32 = 16;

/// The bytes of decoded pictures above which the least recently used ones are dropped.
const TEXCACHE_BUDGET: usize = 64 << 20;

/// The pictures of DS textures decoded so far, keyed by `TexKey`. The key hashes the texture's
/// bytes, palette and format (`texpack::texture_key`), so new contents are new keys and nothing
/// needs invalidating on writes to the texture memory; the cache only drops the least recently
/// used pictures over its byte budget.
struct TexCache {
    map: HashMap<TexKey, (Arc<Image>, u64)>,
    stamp: u64,
    bytes: usize,
}

static TEXCACHE: OnceLock<Mutex<TexCache>> = OnceLock::new();

/// The picture of a DS texture, decoded once and kept (the packs' keys are worked out the same
/// way; the job's own map of keys is used when it has one).
fn decoded(job: &Job, tex: u32, pal: u32) -> Arc<Image> {
    let key = job
        .keys
        .get(&(tex, pal))
        .copied()
        .unwrap_or_else(|| super::texpack::texture_key(&Mem { tex: &job.tex, pal: &job.texpal }, tex, pal));
    let cache = &mut *TEXCACHE
        .get_or_init(|| Mutex::new(TexCache { map: HashMap::new(), stamp: 0, bytes: 0 }))
        .lock()
        .unwrap();
    if let Some((img, stamp)) = cache.map.get_mut(&key) {
        cache.stamp += 1;
        *stamp = cache.stamp;
        return img.clone();
    }
    let img = Arc::new(super::texpack::decode(&Mem { tex: &job.tex, pal: &job.texpal }, tex, pal));
    let size = img.w * img.h * 4;
    cache.stamp += 1;
    cache.map.insert(key, (img.clone(), cache.stamp));
    cache.bytes += size;
    while cache.bytes > TEXCACHE_BUDGET && cache.map.len() > 1 {
        let oldest = cache.map.iter().min_by_key(|(_, (_, s))| *s).map(|(k, _)| *k);
        if let Some(k) = oldest {
            let (img, _) = cache.map.remove(&k).unwrap();
            cache.bytes -= img.w * img.h * 4;
        }
    }
    img
}

/// The job the stage-4 GPU renderer draws: everything the software engine does except the wireframe
/// polygons, the edge marking (bit 5), the widened view and the extras — so opaque, translucent and
/// shadow polygons are all compared. Rendered on the software and the GPU engine, the job gives the
/// same picture to compare (`gpu_parity_check`).
pub(super) fn stage4_job(job: &Job) -> Job {
    let polys = job.polys.iter().filter(|p| (p.attr >> 16) & 31 != 0).copied().collect();
    Job {
        verts: job.verts.clone(),
        polys,
        wbuffer: job.wbuffer,
        disp3dcnt: job.disp3dcnt & !(0x20),
        width: 256,
        clear_color: job.clear_color,
        clear_depth: job.clear_depth,
        clrimage_offset: job.clrimage_offset,
        alpha_ref: job.alpha_ref,
        extras: super::Extras::default(),
        edge: job.edge,
        fog_color: job.fog_color,
        fog_offset: job.fog_offset,
        fog_table: job.fog_table,
        toon: job.toon,
        tex: job.tex.clone(),
        texpal: job.texpal.clone(),
        keys: job.keys.clone(),
        pack: job.pack.clone(),
    }
}

/// The job for stage 7.1-7.2: `stage4_job` with the outline and shading extras on (a
/// representative strength and width/radius), so the outline bits (24-27) and the shade bits
/// (28-31) are part of the comparison.
pub(super) fn stage7_job(job: &Job) -> Job {
    let mut j = stage4_job(job);
    j.extras.outline = 0.5;
    j.extras.outline_width = 0.5;
    j.extras.shadow = 0.5;
    j.extras.shadow_radius = 2.0;
    j.disp3dcnt |= 0x20; // the edge marking (этап 7.3), masked off in `stage4_job`
    j
}

/// The job for stage 7.4: `stage7_job` with the real width kept — the software engine's
/// `extend_sides` fills the columns past the DS's 4:3 on both engines.
pub(super) fn stage7w_job(job: &Job) -> Job {
    let mut j = stage7_job(job);
    j.width = job.width;
    j
}

/// The screen-space edge function, as in the software renderer (positive: the point left of the
/// edge, the inside of a normalized fan triangle).
fn edge(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> f32 {
    (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
}

/// The DS's ownership of an edge pixel: an edge going up, or a flat one going right (the top-left
/// rule in the DS's coordinates).
fn owns(p: (f32, f32), q: (f32, f32)) -> bool {
    let (dy, dx) = (q.1 - p.1, q.0 - p.0);
    dy < 0.0 || (dy == 0.0 && dx > 0.0)
}

/// The fan triangles of a job and their per-triangle data: the rasterized positions (the exact
/// ones pushed a little outward, so the GPU's own coverage never leaves out a pixel the exact test
/// below would keep) and the `Tri` records the fragment shader tests with. The opaque polygons go
/// into one buffer drawn in one pass; each translucent or shadow polygon keeps its own triangles,
/// drawn one at a time in the job's order.
struct Built {
    opaque_verts: Vec<u8>,
    opaque_tris: Vec<u8>,
    serial: Vec<(Vec<u8>, Vec<u8>)>,
}

fn build(job: &Job, s: usize, regions: &HashMap<(u32, u32), TexInfo>) -> Built {
    let sc = s as f32;
    let texture_on = job.disp3dcnt & 1 != 0;
    let mut opaque_verts = Vec::new();
    let mut opaque_tris = Vec::new();
    let mut serial = Vec::new();
    let push = |v: &mut Vec<u8>, f: f32| v.extend_from_slice(&f.to_le_bytes());
    for p in &job.polys {
        let vs = &job.verts[p.first as usize..(p.first + p.n) as usize];
        let mode = (p.attr >> 4) & 3;
        let textured = texture_on && (p.tex >> 26) & 7 != 0;
        let texinfo = textured.then(|| regions.get(&(p.tex, p.pal)).copied()).flatten();
        let vpos: &mut Vec<u8>;
        let tris: &mut Vec<u8>;
        if p.translucent || mode == 3 {
            serial.push((Vec::new(), Vec::new()));
            let last = serial.last_mut().unwrap();
            vpos = &mut last.0;
            tris = &mut last.1;
        } else {
            vpos = &mut opaque_verts;
            tris = &mut opaque_tris;
        }
        // the depth and the interpolation of a vertex, exactly as the software renderer has them
        let depth_of = |v: &GxVertex| -> f32 {
            let w = v.w.max(1) as f32;
            if job.wbuffer { w / 16_777_215.0 } else { ((v.z as f32 / w) * 16384.0 + 16383.0) * 512.0 / 16_777_215.0 }
        };
        for k in 1..vs.len() - 1 {
            let (a, mut b, mut c) = (&vs[0], &vs[k], &vs[k + 1]);
            let (pa, mut pb, mut pc) = ((a.sx * sc, a.sy * sc), (b.sx * sc, b.sy * sc), (c.sx * sc, c.sy * sc));
            let area = edge(pa, pb, pc);
            if area == 0.0 {
                continue;
            }
            if area < 0.0 {
                std::mem::swap(&mut b, &mut c);
                std::mem::swap(&mut pb, &mut pc);
            }
            let area = area.abs();
            // the per-vertex data follows the (possibly swapped) order
            let pack = |v: &GxVertex, t: &mut Vec<u8>| {
                let w = v.w.max(1) as f32;
                let iw = 1.0 / w;
                push(t, iw);
                push(t, depth_of(v));
                push(t, v.r);
                push(t, v.g);
                push(t, v.b);
                push(t, v.s);
                push(t, v.t);
            };
            push(tris, pa.0);
            push(tris, pa.1);
            push(tris, pb.0);
            push(tris, pb.1);
            push(tris, pc.0);
            push(tris, pc.1);
            push(tris, area);
            let o = (owns(pb, pc) as u32) | (owns(pc, pa) as u32) << 1 | (owns(pa, pb) as u32) << 2;
            tris.extend_from_slice(&o.to_le_bytes());
            tris.extend_from_slice(&mode.to_le_bytes());
            pack(a, tris);
            pack(b, tris);
            pack(c, tris);
            // the texture: where its picture sits in the atlas, and its coordinates (an all-zero
            // record with `w` 0 is no texture)
            let ti = texinfo.unwrap_or(TexInfo { ux: 0, uy: 0, w: 0, h: 0, ow: 0.0, oh: 0.0, flags: 0 });
            for u in [ti.ux, ti.uy, ti.w, ti.h] {
                tris.extend_from_slice(&u.to_le_bytes());
            }
            push(tris, ti.ow);
            push(tris, ti.oh);
            tris.extend_from_slice(&ti.flags.to_le_bytes());
            // the polygon's bits: fog (bit 15), alpha (16-20), ID (24-29), the equal-depth test
            // (bit 14) and whether a translucent polygon writes the depth (bit 11)
            tris.extend_from_slice(&((p.attr & 0x8000 != 0) as u32).to_le_bytes());
            tris.extend_from_slice(&((p.attr >> 16) & 31).to_le_bytes());
            tris.extend_from_slice(&((p.attr >> 24) & 63).to_le_bytes());
            tris.extend_from_slice(&((p.attr & 0x4000 != 0) as u32).to_le_bytes());
            tris.extend_from_slice(&((p.attr & 0x800 != 0) as u32).to_le_bytes());
            // the rasterized triangle: each edge pushed 0.5 px outward (an exact superset of the
            // pixels the software engine keeps; the fragment shader tests the exact edges)
            let (ia, ib, ic) = (
                (pa.0 as f64, pa.1 as f64),
                (pb.0 as f64, pb.1 as f64),
                (pc.0 as f64, pc.1 as f64),
            );
            let line = |p: (f64, f64), q: (f64, f64)| -> (f64, f64, f64) {
                (-(q.1 - p.1), q.0 - p.0, (q.1 - p.1) * p.0 - (q.0 - p.0) * p.1)
            };
            let shift = |l: (f64, f64, f64)| {
                let len = (l.0 * l.0 + l.1 * l.1).sqrt();
                (l.0, l.1, l.2 + 0.5 * len)
            };
            let (l0, l1, l2) = (shift(line(ib, ic)), shift(line(ic, ia)), shift(line(ia, ib)));
            let meet = |l: (f64, f64, f64), m: (f64, f64, f64)| -> Option<(f64, f64)> {
                let det = l.0 * m.1 - m.0 * l.1;
                if det.abs() < 1e-12 {
                    None
                } else {
                    Some(((l.1 * m.2 - m.1 * l.2) / det, (l.2 * m.0 - m.2 * l.0) / det))
                }
            };
            let va = meet(l1, l2).unwrap_or(ia);
            let vb = meet(l2, l0).unwrap_or(ib);
            let vc = meet(l0, l1).unwrap_or(ic);
            for v in [va, vb, vc] {
                push(vpos, v.0 as f32);
                push(vpos, v.1 as f32);
            }
        }
    }
    Built { opaque_verts, opaque_tris, serial }
}

/// A frame the GPU drew, with the picture and the auxiliary maps the outline and the shading read
/// (этапы 7.0 и 5.1 of `docs/GPU-BACKEND.md`).
struct Rendered {
    words: Vec<u32>,
    oid: Vec<u8>,
    cover: Vec<u8>,
    trans: Vec<u8>,
    depth: Vec<f32>,
    notes: u32,
}

/// Draws a job as stage 4 can and reads the picture back as the same words the software renderer
/// returns, with its `notes` mask. The frame is drawn as: the clear and the rear plane, then the
/// opaque polygons in one pass (hardware depth test), then each translucent (or shadow) polygon in
/// its own pass in the job's order — the depth test in the shader against the accumulated depth,
/// the shadow mask's mark, then the DS blend `(src*(a+1) + dst*(31-a))/32` over what is there —
/// then the fog over the finished picture, which also drops the alpha-0 pixels.
pub(super) fn render(job: &Job, scale: usize) -> Result<(Vec<u32>, u32), String> {
    let s = scale.max(1);
    // supersampling like the software engine's (этап 6.0): the frame k times as large each way,
    // then the k x k samples of a pixel averaged
    let k = render3d::msaa_factor(job, s);
    if k >= 2 {
        let r = render_all(job, s * k, false, k)?;
        return Ok((r.words, r.notes & !(1 << 1)));
    }
    let r = render_all(job, scale, false, 1)?;
    Ok((r.words, r.notes))
}

/// Draws a job and reads the auxiliary maps back too (этапы 7.0 и 5.1 of `docs/GPU-BACKEND.md`):
/// the words, `oid`, `cover`, `trans` and the depth.
pub(super) fn render_maps(job: &Job, scale: usize) -> Result<(Vec<u32>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<f32>, u32), String> {
    let r = render_all(job, scale, true, 1)?;
    Ok((r.words, r.oid, r.cover, r.trans, r.depth, r.notes))
}

/// `maps`: read the auxiliary maps back too (the parity harness wants them; a frame for the window
/// needs only the words, and four more readbacks of the picture's size cost about as much as the
/// drawing — этап 6.2.2).
///
/// `k` > 1: `scale` is already the supersampled one (s·k) and the words come back resolved to the
/// size of `scale / k` (the GPU averages the k x k samples, этап 6.2.1); `maps` must be off then.
fn render_all(job: &Job, scale: usize, maps: bool, k: usize) -> Result<Rendered, String> {
    let t0 = std::time::Instant::now();
    let c = ctx()?;
    let s = scale.max(1);
    let (w, h) = (job.width * s, 192 * s);
    let mut notes = 0u32;
    if job.disp3dcnt & 0x10 != 0 {
        notes |= 1 << 1;
    }
    // the clear word and the fog's colour as one word
    let (cr, cg, cb) = render3d::rgb555((job.clear_color & 0x7FFF) as u16);
    let calpha = (job.clear_color >> 16) & 31;
    let clear_word = if calpha == 0 { 0 } else { cr | cg << 6 | cb << 12 | calpha << 18 };
    let (fr, fg, fb) = render3d::rgb555((job.fog_color & 0x7FFF) as u16);
    let fog_color = fr | fg << 6 | fb << 12 | ((job.fog_color >> 16) & 31) << 18;
    // the frame's textures decoded into one atlas, and where each picture sits in it
    let texture_on = job.disp3dcnt & 1 != 0;
    let mut unique: Vec<(u32, u32)> = Vec::new();
    for p in &job.polys {
        if texture_on && (p.tex >> 26) & 7 != 0 && !unique.contains(&(p.tex, p.pal)) {
            unique.push((p.tex, p.pal));
        }
    }
    let mut pics: Vec<((u32, u32), Arc<Image>, f32, f32, u32)> = Vec::new();
    for &(tex, pal) in &unique {
        let key = job.keys.get(&(tex, pal));
        match key.and_then(|k| job.pack.get(k)) {
            Some(img) => {
                let (ow, oh) = ((8u32 << ((tex >> 20) & 7)) as f32, (8u32 << ((tex >> 23) & 7)) as f32);
                pics.push(((tex, pal), img, ow, oh, F_SCALED));
            }
            None => pics.push(((tex, pal), decoded(job, tex, pal), 0.0, 0.0, 0)),
        }
    }
    // a shelf: one row at a time, the tallest pictures first
    pics.sort_by(|a, b| b.1.h.cmp(&a.1.h).then(b.1.w.cmp(&a.1.w)));
    take_made(); // this frame's counts only
    let device = &c.device;
    // the same pictures in the same order make the same atlas as the last frame's: it is kept (этап 9.1.6)
    let atlas_key: Vec<(u32, u32, usize, u32, u32, u32)> = pics.iter().map(|((tex, pal), img, ow, oh, scaled)| (*tex, *pal, Arc::as_ptr(img) as usize, ow.to_bits(), oh.to_bits(), *scaled)).collect();
    let cached = c.atlas_cache.lock().unwrap().as_ref().filter(|e| e.key == atlas_key).cloned();
    let atlas: Arc<AtlasEntry> = match cached {
        Some(e) => {
            made(7, 1);
            e
        }
        None => {
        let atlas_w = pics.iter().map(|p| p.1.w as u32).max().unwrap_or(1).max(1);
        let mut regions: HashMap<(u32, u32), TexInfo> = HashMap::new();
        let (mut x, mut y, mut row_h) = (0u32, 0u32, 0u32);
        for ((tex, pal), img, ow, oh, scaled) in &pics {
            let (pw, ph) = (img.w as u32, img.h as u32);
            if x + pw > atlas_w {
                y += row_h;
                x = 0;
                row_h = 0;
            }
            let flags = *scaled
                | ((tex & 0x10000 != 0) as u32) * F_REPEAT_S
                | ((tex & 0x20000 != 0) as u32) * F_REPEAT_T
                | ((tex & 0x40000 != 0) as u32) * F_FLIP_S
                | ((tex & 0x80000 != 0) as u32) * F_FLIP_T;
            regions.insert((*tex, *pal), TexInfo { ux: x, uy: y, w: pw, h: ph, ow: *ow, oh: *oh, flags });
            x += pw;
            row_h = row_h.max(ph);
        }
        let atlas_h = (y + row_h).max(1);

        let max_dim = device.limits().max_texture_dimension_2d;
        if atlas_w > max_dim || atlas_h > max_dim {
            return Err(format!("the frame's textures make a {atlas_w}x{atlas_h} atlas, over the device's {max_dim}"));
        }
        let atlas = made_texture(device, &wgpu::TextureDescriptor {
            label: Some("3d texture atlas"),
            size: wgpu::Extent3d { width: atlas_w, height: atlas_h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for ((tex, pal), img, _, _, _) in &pics {
            let r = regions[&(*tex, *pal)];
            made(5, img.rgba.len() as u64);
            c.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &atlas,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: r.ux, y: r.uy, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &img.rgba,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(r.w * 4), rows_per_image: None },
                wgpu::Extent3d { width: r.w, height: r.h, depth_or_array_layers: 1 },
            );
        }

            let e = Arc::new(AtlasEntry { key: atlas_key, tex: atlas, regions, _pics: pics.iter().map(|p| p.1.clone()).collect() });
            *c.atlas_cache.lock().unwrap() = Some(e.clone());
            e
        }
    };
    let regions = &atlas.regions;
    let t_atlas = t0.elapsed();
    let built = build(job, s, regions);
    let t_build = t0.elapsed();
    let rear = rear_data(job);
    // the uniforms: sizes, the clear word, the modes, the scale, the rear plane's origin, the fog,
    // the clear ID, the blend flag, the widened view's settings, and the toon and fog tables
    let fog_step = (0x400u32 >> ((job.disp3dcnt >> 8) & 15)).max(1);
    let rays = super::render3d::shade_rays();
    let clear_depth = render3d::clear_depth(job.clear_depth);
    let mut uni = Vec::with_capacity(108 * 4);
    for u in [
        w as u32,
        h as u32,
        clear_word,
        job.wbuffer as u32,
        (job.disp3dcnt & 4 != 0) as u32,
        job.alpha_ref as u32,
        (job.disp3dcnt & 2 != 0) as u32,
        matches!(job.extras.filter, super::TexFilter::Bilinear) as u32,
        s as u32,
        (job.clrimage_offset & 0xFF) as u32,
        (job.clrimage_offset >> 8) as u32,
        (job.disp3dcnt & 0x80 != 0) as u32,
        (job.disp3dcnt & 0x40 != 0) as u32,
        fog_color,
        fog_step,
        job.fog_offset as u32,
        (job.clear_color >> 24) & 63,
        (job.disp3dcnt & 8 != 0) as u32,
        job.extras.outline.to_bits(),
        job.extras.outline_width.to_bits(),
        job.extras.shadow.to_bits(),
        ((job.extras.shadow_radius * s as f32).round() as u32).max(2),
        rays[0].0.to_bits(),
        rays[0].1.to_bits(),
        rays[1].0.to_bits(),
        rays[1].1.to_bits(),
        rays[2].0.to_bits(),
        rays[2].1.to_bits(),
        (job.disp3dcnt & 0x20 != 0) as u32,
        job.edge[0] as u32 | (job.edge[1] as u32) << 16,
        job.edge[2] as u32 | (job.edge[3] as u32) << 16,
        job.edge[4] as u32 | (job.edge[5] as u32) << 16,
        job.edge[6] as u32 | (job.edge[7] as u32) << 16,
        (job.width > 256 && std::env::var_os("SUNPETAL_NO_EXTEND").is_none()) as u32,
        ((job.width.saturating_sub(256)) / 2 * s) as u32,
        (256 * s) as u32,
        clear_depth.to_bits(),
    ] {
        uni.extend_from_slice(&u.to_le_bytes());
    }
    // 37 scalars = 148 bytes: pad to the toon table's alignment (160)
    uni.extend_from_slice(&[0u8; 12]);
    for t in job.toon {
        uni.extend_from_slice(&(t as u32).to_le_bytes());
    }
    for f in job.fog_table {
        uni.extend_from_slice(&(f as u32).to_le_bytes());
    }
    let ub = frame_buffer(c, "3d uniforms", &uni, wgpu::BufferUsages::UNIFORM);
    // a full-screen triangle (big enough to cover the viewport) for the rear plane and the fog
    let mut qp = Vec::with_capacity(24);
    for v in [(0.0f32, h as f32), (2.0 * w as f32, h as f32), (0.0f32, -(h as f32))] {
        qp.extend_from_slice(&v.0.to_le_bytes());
        qp.extend_from_slice(&v.1.to_le_bytes());
    }
    let quad = frame_buffer(c, "3d fullscreen", &qp, wgpu::BufferUsages::VERTEX);
    // the accumulated picture, ping-ponged between the translucent polygons: the word (colour and
    // alpha), the polygon ID, the shadow mask and the depth; the fog flag is written straight
    let ping = wgpu::TextureUsages::RENDER_ATTACHMENT
        | wgpu::TextureUsages::TEXTURE_BINDING
        | wgpu::TextureUsages::COPY_SRC
        | wgpu::TextureUsages::COPY_DST;
    let pair = |fmt: wgpu::TextureFormat| {
        [attach(c, w, h, fmt, ping), attach(c, w, h, fmt, ping)]
    };
    let word = pair(wgpu::TextureFormat::R32Uint);
    let id = pair(wgpu::TextureFormat::R32Uint);
    let stencil = pair(wgpu::TextureFormat::R32Uint);
    let depth = pair(wgpu::TextureFormat::R32Float);
    let oid = pair(wgpu::TextureFormat::R32Uint);
    let cover = pair(wgpu::TextureFormat::R32Uint);
    let trans = pair(wgpu::TextureFormat::R32Uint);
    let fog = attach(c, w, h, wgpu::TextureFormat::R32Uint, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
    let depth_stencil = attach(c, w, h, wgpu::TextureFormat::Depth32Float, wgpu::TextureUsages::RENDER_ATTACHMENT);
    let fin = attach(c, w, h, wgpu::TextureFormat::R32Uint, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING);
    let mut cur = 0usize;
    let t_res = t0.elapsed();
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let rear_bind = rear.as_ref().map(|data| {
        let rt = made_texture(device, &wgpu::TextureDescriptor {
            label: Some("3d rear bitmap"),
            size: wgpu::Extent3d { width: 256, height: 256, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        made(5, data.len() as u64);
        c.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &rt, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256 * 4 * 4), rows_per_image: None },
            wgpu::Extent3d { width: 256, height: 256, depth_or_array_layers: 1 },
        );
        let rv = rt.create_view(&Default::default());
        made_bind_group(device, &wgpu::BindGroupDescriptor {
            label: Some("3d rear"),
            layout: &c.bgl_rear,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&rv) },
            ],
        })
    });
    let atlas_view = atlas.tex.create_view(&Default::default());
    // the clear plane and the opaque polygons, one pass with the hardware depth test
    {
        let wv = word[0].create_view(&Default::default());
        let iv = id[0].create_view(&Default::default());
        let sv = stencil[0].create_view(&Default::default());
        let fv = fog.create_view(&Default::default());
        let dv = depth[0].create_view(&Default::default());
        let ov = oid[0].create_view(&Default::default());
        let cov = cover[0].create_view(&Default::default());
        let trv = trans[0].create_view(&Default::default());
        let dsv = depth_stencil.create_view(&Default::default());
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3d geometry"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {
                    view: &wv,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: clear_word as f64, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &fv,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: (job.clear_color & 0x8000 != 0) as u32 as f64, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &dv,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: clear_depth as f64, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &iv,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: ((job.clear_color >> 24) & 63) as f64, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &sv,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &ov,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: ((job.clear_color >> 24) & 63) as f64, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &cov,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &trv,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
                }),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &dsv,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(clear_depth as f32), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
        if let Some(bind) = &rear_bind {
            rp.set_pipeline(&c.rear_pipeline);
            rp.set_bind_group(0, bind, &[]);
            rp.set_vertex_buffer(0, quad.slice(..));
            rp.draw(0..3, 0..1);
        }
        let nvert = (built.opaque_verts.len() / 8) as u32;
        if nvert > 0 {
            let vb = made_buffer_init(device, &wgpu::util::BufferInitDescriptor {
                label: Some("3d verts"),
                contents: &built.opaque_verts,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
            let tb = made_buffer_init(device, &wgpu::util::BufferInitDescriptor {
                label: Some("3d tris"),
                contents: &built.opaque_tris,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });
            let bind = made_bind_group(device, &wgpu::BindGroupDescriptor {
                label: Some("3d"),
                layout: &c.bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: tb.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&atlas_view) },
                ],
            });
            rp.set_pipeline(&c.pipeline);
            rp.set_bind_group(0, &bind, &[]);
            rp.set_vertex_buffer(0, vb.slice(..));
            rp.draw(0..nvert, 0..1);
        }
    }
    // the translucent (and shadow) polygons, each in its own pass, in the job's order
    let serial_masks: Vec<bool> = job
        .polys
        .iter()
        .filter(|p| p.translucent || (p.attr >> 4) & 3 == 3)
        .map(|p| (p.attr >> 4) & 3 == 3 && (p.attr >> 24) & 63 == 0)
        .collect();
    // the two buffers hold the same picture before each polygon; a polygon reads one and draws
    // into the other inside its bounding box only, then the box is copied back (этап 8.2.4: a
    // whole-picture copy of seven textures per polygon cost most of a busy frame)
    let (mut n_blend, mut n_mask, mut n_skipped, mut px_copied) = (0u32, 0u32, 0u32, 0u64);
    if !built.serial.is_empty() {
        let next = 1 - cur;
        for (a, b) in [
            (&word[cur], &word[next]),
            (&id[cur], &id[next]),
            (&stencil[cur], &stencil[next]),
            (&depth[cur], &depth[next]),
            (&oid[cur], &oid[next]),
            (&cover[cur], &cover[next]),
            (&trans[cur], &trans[next]),
        ] {
            enc.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo { texture: a, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyTextureInfo { texture: b, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
            );
        }
    }
    // each polygon's box, buffers and kind; then runs of consecutive polygons of one kind whose
    // boxes do not overlap share a pass (they read the one buffer and write the other, so the
    // order between them cannot matter; the passes cost far more than the polygons)
    struct Item {
        x0: u32,
        y0: u32,
        bw: u32,
        bh: u32,
        nvert: u32,
        mask: bool,
        /// Its index in `built.serial`.
        src: usize,
        vb: wgpu::Buffer,
        tb: wgpu::Buffer,
    }
    let mut items: Vec<Item> = Vec::new();
    for (i, (verts, tris)) in built.serial.iter().enumerate() {
        let nvert = (verts.len() / 8) as u32;
        if nvert == 0 {
            continue; // a degenerate polygon: no triangles to draw
        }
        // the box of the rasterized triangles (their vertices are in picture pixels), a pixel wider
        let (mut bx0, mut by0, mut bx1, mut by1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for v in verts.chunks_exact(8) {
            let x = f32::from_le_bytes([v[0], v[1], v[2], v[3]]);
            let y = f32::from_le_bytes([v[4], v[5], v[6], v[7]]);
            bx0 = bx0.min(x);
            bx1 = bx1.max(x);
            by0 = by0.min(y);
            by1 = by1.max(y);
        }
        let x0 = ((bx0.floor() - 1.0).max(0.0)) as u32;
        let y0 = ((by0.floor() - 1.0).max(0.0)) as u32;
        let x1 = ((bx1.ceil() + 1.0).min(w as f32)).max(0.0) as u32;
        let y1 = ((by1.ceil() + 1.0).min(h as f32)).max(0.0) as u32;
        if x1 <= x0 || y1 <= y0 {
            n_skipped += 1;
            continue; // off the picture: it changes nothing
        }
        px_copied += ((x1 - x0) * (y1 - y0)) as u64;
        let vb = made_buffer_init(device, &wgpu::util::BufferInitDescriptor {
            label: Some("3d trans verts"),
            contents: verts,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let tb = made_buffer_init(device, &wgpu::util::BufferInitDescriptor {
            label: Some("3d trans tris"),
            contents: tris,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        items.push(Item { x0, y0, bw: x1 - x0, bh: y1 - y0, nvert, mask: serial_masks[i], src: i, vb, tb });
    }
    if std::env::var_os("SUNPETAL_GPU_TIMING").is_some() && !items.is_empty() {
        // how many translucent/shadow polygons' boxes meet in one 16x16 tile (этап 9.4.0)
        let (tw, th) = (w.div_ceil(16), h.div_ceil(16));
        let mut count = vec![0u32; tw * th];
        for it in &items {
            for ty in (it.y0 / 16) as usize..=((it.y0 + it.bh - 1) / 16) as usize {
                for tx in (it.x0 / 16) as usize..=((it.x0 + it.bw - 1) / 16) as usize {
                    count[ty * tw + tx] += 1;
                }
            }
        }
        let hit: Vec<u32> = count.iter().copied().filter(|&n| n > 0).collect();
        let (max, sum) = (hit.iter().copied().max().unwrap_or(0), hit.iter().map(|&n| n as u64).sum::<u64>());
        let over = hit.iter().filter(|&&n| n > 32).count();
        eprintln!("  tiles 16x16: {} of {} hit, polygons per tile avg {:.1}, max {max}, {over} with more than 32", hit.len(), tw * th, sum as f64 / hit.len().max(1) as f64);
    }
    // a polygon's level: one past the highest level of an earlier polygon whose box overlaps it
    // (those must come first, in order). Polygons of one level overlap nobody of that level, so
    // they read the one buffer and write the other in any order: a level's masks go in one pass,
    // its blends in another (the passes cost far more than the polygons)
    let overlap = |o: &Item, n: &Item| !(n.x0 >= o.x0 + o.bw || o.x0 >= n.x0 + n.bw || n.y0 >= o.y0 + o.bh || o.y0 >= n.y0 + n.bh);
    let mut level = vec![0usize; items.len()];
    let batching = std::env::var_os("SUNPETAL_NO_BATCH").is_none();
    for j in 0..items.len() {
        level[j] = if batching { (0..j).filter(|&i| overlap(&items[i], &items[j])).map(|i| level[i] + 1).max().unwrap_or(0) } else { j };
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for l in 0..=level.iter().copied().max().unwrap_or(0) {
        for mask in [true, false] {
            let g: Vec<usize> = (0..items.len()).filter(|&j| level[j] == l && items[j].mask == mask).collect();
            if !g.is_empty() {
                groups.push(g);
            }
        }
    }
    // the tiled pass (этап 9.4, `SUNPETAL_TILED`): all the polygons in one full-screen pass, each pixel going
    // through its 16x16 tile's polygons in the job's order, instead of a pass per group
    let tiled = std::env::var_os("SUNPETAL_TILED").is_none_or(|v| v != "0") && !items.is_empty();
    if tiled {
        let next = 1 - cur;
        let mut all_tris: Vec<u8> = Vec::new();
        let mut table: Vec<u8> = Vec::new();
        let mut first = 0u32;
        for it in &items {
            let n = it.nvert / 3;
            all_tris.extend_from_slice(&built.serial[it.src].1);
            for v in [first, n, it.mask as u32, 0] {
                table.extend_from_slice(&v.to_le_bytes());
            }
            first += n;
        }
        let (tw, th) = (w.div_ceil(16), h.div_ceil(16));
        let mut lists: Vec<Vec<u32>> = vec![Vec::new(); tw * th];
        let (mut ux0, mut uy0, mut ux1, mut uy1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        for (j, it) in items.iter().enumerate() {
            for ty in (it.y0 / 16) as usize..=((it.y0 + it.bh - 1) / 16) as usize {
                for tx in (it.x0 / 16) as usize..=((it.x0 + it.bw - 1) / 16) as usize {
                    lists[ty * tw + tx].push(j as u32);
                }
            }
            (ux0, uy0, ux1, uy1) = (ux0.min(it.x0), uy0.min(it.y0), ux1.max(it.x0 + it.bw), uy1.max(it.y0 + it.bh));
        }
        let (mut off, mut idx) = (Vec::with_capacity((tw * th + 1) * 4), Vec::new());
        off.extend_from_slice(&0u32.to_le_bytes());
        let mut n_idx = 0u32;
        for l in &lists {
            for &j in l {
                idx.extend_from_slice(&j.to_le_bytes());
            }
            n_idx += l.len() as u32;
            off.extend_from_slice(&n_idx.to_le_bytes());
        }
        let buf = |label: &str, contents: &[u8]| made_buffer_init(device, &wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage: wgpu::BufferUsages::STORAGE });
        let (tris_b, polys_b, off_b, idx_b) = (buf("3d tiled tris", &all_tris), buf("3d tiled polys", &table), buf("3d tiled tile offsets", &off), buf("3d tiled tile indices", &idx));
        let fog_old = attach(c, w, h, wgpu::TextureFormat::R32Uint, wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING);
        enc.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: &fog, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: &fog_old, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        );
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let (wv, iv, sv, dv, ov, cov, trv, fv) = (view(&word[next]), view(&id[next]), view(&stencil[next]), view(&depth[next]), view(&oid[next]), view(&cover[next]), view(&trans[next]), view(&fog));
        let (wc, ic, sc, dc, oc, coc, trc, fo) = (view(&word[cur]), view(&id[cur]), view(&stencil[cur]), view(&depth[cur]), view(&oid[cur]), view(&cover[cur]), view(&trans[cur]), view(&fog_old));
        let bind = made_bind_group(device, &wgpu::BindGroupDescriptor {
            label: Some("3d tiled"),
            layout: &c.bgl_tiled,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: tris_b.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&atlas_view) },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&wc) },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&ic) },
                wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&sc) },
                wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(&dc) },
                wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(&oc) },
                wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(&coc) },
                wgpu::BindGroupEntry { binding: 13, resource: wgpu::BindingResource::TextureView(&trc) },
                wgpu::BindGroupEntry { binding: 17, resource: polys_b.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 18, resource: off_b.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 19, resource: idx_b.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 20, resource: wgpu::BindingResource::TextureView(&fo) },
            ],
        });
        let load = |v| Some(wgpu::RenderPassColorAttachment { view: v, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } });
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3d tiled translucent"),
                color_attachments: &[load(&wv), load(&fv), load(&dv), load(&iv), load(&sv), load(&ov), load(&cov), load(&trv)],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
            rp.set_scissor_rect(ux0, uy0, ux1 - ux0, uy1 - uy0);
            rp.set_pipeline(&c.tiled_pipeline);
            rp.set_bind_group(0, &bind, &[]);
            rp.set_vertex_buffer(0, quad.slice(..));
            rp.draw(0..3, 0..1);
        }
        n_blend += 1;
        for (from, to) in [
            (&word[next], &word[cur]),
            (&id[next], &id[cur]),
            (&stencil[next], &stencil[cur]),
            (&depth[next], &depth[cur]),
            (&oid[next], &oid[cur]),
            (&cover[next], &cover[cur]),
            (&trans[next], &trans[cur]),
        ] {
            enc.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo { texture: from, mip_level: 0, origin: wgpu::Origin3d { x: ux0, y: uy0, z: 0 }, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyTextureInfo { texture: to, mip_level: 0, origin: wgpu::Origin3d { x: ux0, y: uy0, z: 0 }, aspect: wgpu::TextureAspect::All },
                wgpu::Extent3d { width: ux1 - ux0, height: uy1 - uy0, depth_or_array_layers: 1 },
            );
        }
    }
    for group in groups.iter().filter(|_| !tiled) {
        let batch: Vec<&Item> = group.iter().map(|&j| &items[j]).collect();
        let mask = batch[0].mask;
        let next = 1 - cur;
        if mask {
            n_mask += 1;
            let sv = stencil[next].create_view(&Default::default());
            let dc = depth[cur].create_view(&Default::default());
            let binds: Vec<wgpu::BindGroup> = batch
                .iter()
                .map(|it| {
                    made_bind_group(device, &wgpu::BindGroupDescriptor {
                        label: Some("3d mask"),
                        layout: &c.bgl_mask,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: it.tb.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(&dc) },
                        ],
                    })
                })
                .collect();
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3d shadow mask"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &sv,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
            rp.set_pipeline(&c.mask_pipeline);
            for (it, bind) in batch.iter().zip(&binds) {
                rp.set_scissor_rect(it.x0, it.y0, it.bw, it.bh);
                rp.set_bind_group(0, bind, &[]);
                rp.set_vertex_buffer(0, it.vb.slice(..));
                rp.draw(0..it.nvert, 0..1);
            }
        } else {
            n_blend += 1;
            let wv = word[next].create_view(&Default::default());
            let iv = id[next].create_view(&Default::default());
            let sv = stencil[next].create_view(&Default::default());
            let fv = fog.create_view(&Default::default());
            let dv = depth[next].create_view(&Default::default());
            let ov = oid[next].create_view(&Default::default());
            let cov = cover[next].create_view(&Default::default());
            let trv = trans[next].create_view(&Default::default());
            let wc = word[cur].create_view(&Default::default());
            let ic = id[cur].create_view(&Default::default());
            let sc = stencil[cur].create_view(&Default::default());
            let dc = depth[cur].create_view(&Default::default());
            let oc = oid[cur].create_view(&Default::default());
            let coc = cover[cur].create_view(&Default::default());
            let trc = trans[cur].create_view(&Default::default());
            let binds: Vec<wgpu::BindGroup> = batch
                .iter()
                .map(|it| {
                    made_bind_group(device, &wgpu::BindGroupDescriptor {
                        label: Some("3d blend"),
                        layout: &c.bgl_blend,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: it.tb.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&atlas_view) },
                            wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&wc) },
                            wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&ic) },
                            wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&sc) },
                            wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(&dc) },
                            wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(&oc) },
                            wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(&coc) },
                            wgpu::BindGroupEntry { binding: 13, resource: wgpu::BindingResource::TextureView(&trc) },
                        ],
                    })
                })
                .collect();
            let load = |v| Some(wgpu::RenderPassColorAttachment { view: v, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } });
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3d translucent"),
                color_attachments: &[load(&wv), load(&fv), load(&dv), load(&iv), load(&sv), load(&ov), load(&cov), load(&trv)],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
            rp.set_pipeline(&c.blend_pipeline);
            for (it, bind) in batch.iter().zip(&binds) {
                rp.set_scissor_rect(it.x0, it.y0, it.bw, it.bh);
                rp.set_bind_group(0, bind, &[]);
                rp.set_vertex_buffer(0, it.vb.slice(..));
                rp.draw(0..it.nvert, 0..1);
            }
        }
        // each box back into the other buffer, so both hold the picture again
        let changed: Vec<(&wgpu::Texture, &wgpu::Texture)> = if mask {
            vec![(&stencil[next], &stencil[cur])]
        } else {
            vec![
                (&word[next], &word[cur]),
                (&id[next], &id[cur]),
                (&stencil[next], &stencil[cur]),
                (&depth[next], &depth[cur]),
                (&oid[next], &oid[cur]),
                (&cover[next], &cover[cur]),
                (&trans[next], &trans[cur]),
            ]
        };
        for it in &batch {
            for &(from, to) in &changed {
                enc.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo { texture: from, mip_level: 0, origin: wgpu::Origin3d { x: it.x0, y: it.y0, z: 0 }, aspect: wgpu::TextureAspect::All },
                    wgpu::TexelCopyTextureInfo { texture: to, mip_level: 0, origin: wgpu::Origin3d { x: it.x0, y: it.y0, z: 0 }, aspect: wgpu::TextureAspect::All },
                    wgpu::Extent3d { width: it.bw, height: it.bh, depth_or_array_layers: 1 },
                );
            }
        }
    }
    // the widened view (этап 7.4): the software engine's `extend_sides` — the side columns no
    // polygon touched run on with the row's first sky column (else the nearest row's sky, else the
    // row's first far column). Three passes: the source per row and side, the nearest sky rows,
    // and the fill over the whole picture (the untouched pixels carry over).
    if job.width > 256 && std::env::var_os("SUNPETAL_NO_EXTEND").is_none() {
        let src0 = attach(c, 2, h, wgpu::TextureFormat::Rgba32Uint, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        let src1 = attach(c, 2, h, wgpu::TextureFormat::Rgba32Uint, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        // pass 1: per row and side the first sky column in from the inner edge, else the first far
        {
            let dv = depth[cur].create_view(&Default::default());
            let s0v = src0.create_view(&Default::default());
            let bind = made_bind_group(device, &wgpu::BindGroupDescriptor {
                label: Some("3d wide rows"),
                layout: &c.bgl_rows,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&dv) },
                ],
            });
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3d wide rows"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &s0v,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&c.rows_pipeline);
            rp.set_bind_group(0, &bind, &[]);
            rp.set_vertex_buffer(0, quad.slice(..));
            rp.set_viewport(0.0, 0.0, 2.0, h as f32, 0.0, 1.0);
            rp.draw(0..3, 0..1);
        }
        // pass 2: per row and side the nearest row with a sky column, above first
        {
            let s0v = src0.create_view(&Default::default());
            let s1v = src1.create_view(&Default::default());
            let bind = made_bind_group(device, &wgpu::BindGroupDescriptor {
                label: Some("3d wide sky rows"),
                layout: &c.bgl_skyrow,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 16, resource: wgpu::BindingResource::TextureView(&s0v) },
                ],
            });
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3d wide sky rows"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &s1v,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&c.skyrow_pipeline);
            rp.set_bind_group(0, &bind, &[]);
            rp.set_vertex_buffer(0, quad.slice(..));
            rp.set_viewport(0.0, 0.0, 2.0, h as f32, 0.0, 1.0);
            rp.draw(0..3, 0..1);
        }
        // pass 3: the fill into the other ping-pong buffer (the fog through a copy, it has no pair)
        let next = 1 - cur;
        let fog_src = attach(c, w, h, wgpu::TextureFormat::R32Uint, wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING);
        enc.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: &fog, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: &fog_src, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        );
        {
            let s1v = src1.create_view(&Default::default());
            let wv = word[next].create_view(&Default::default());
            let fv = fog.create_view(&Default::default());
            let dv = depth[next].create_view(&Default::default());
            let iv = id[next].create_view(&Default::default());
            let ov = oid[next].create_view(&Default::default());
            let cov = cover[next].create_view(&Default::default());
            let trv = trans[next].create_view(&Default::default());
            let wc = word[cur].create_view(&Default::default());
            let fsc = fog_src.create_view(&Default::default());
            let dc = depth[cur].create_view(&Default::default());
            let ic = id[cur].create_view(&Default::default());
            let oc = oid[cur].create_view(&Default::default());
            let coc = cover[cur].create_view(&Default::default());
            let trc = trans[cur].create_view(&Default::default());
            let bind = made_bind_group(device, &wgpu::BindGroupDescriptor {
                label: Some("3d wide"),
                layout: &c.bgl_extend,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&wc) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&fsc) },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&dc) },
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&ic) },
                    wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(&oc) },
                    wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(&coc) },
                    wgpu::BindGroupEntry { binding: 13, resource: wgpu::BindingResource::TextureView(&trc) },
                    wgpu::BindGroupEntry { binding: 16, resource: wgpu::BindingResource::TextureView(&s1v) },
                ],
            });
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3d wide"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment { view: &wv, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store } }),
                    Some(wgpu::RenderPassColorAttachment { view: &fv, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store } }),
                    Some(wgpu::RenderPassColorAttachment { view: &dv, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store } }),
                    Some(wgpu::RenderPassColorAttachment { view: &iv, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store } }),
                    Some(wgpu::RenderPassColorAttachment { view: &ov, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store } }),
                    Some(wgpu::RenderPassColorAttachment { view: &cov, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store } }),
                    Some(wgpu::RenderPassColorAttachment { view: &trv, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store } }),
                ],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&c.extend_pipeline);
            rp.set_bind_group(0, &bind, &[]);
            rp.set_vertex_buffer(0, quad.slice(..));
            rp.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
            rp.draw(0..3, 0..1);
        }
        cur = next;
    }
    // the outline (этап 7.1): the software engine's `outline_pass` over the finished picture, a
    // line level (0-15) per pixel, before the fog changes the alpha
    let outline_tex = attach(c, w, h, wgpu::TextureFormat::R32Uint, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
    {
        let ov = outline_tex.create_view(&Default::default());
        let wv = word[cur].create_view(&Default::default());
        let dv = depth[cur].create_view(&Default::default());
        let oiv = oid[cur].create_view(&Default::default());
        let cov = cover[cur].create_view(&Default::default());
        let trv = trans[cur].create_view(&Default::default());
        let bind = made_bind_group(device, &wgpu::BindGroupDescriptor {
            label: Some("3d outline"),
            layout: &c.bgl_outline,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&wv) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&dv) },
                wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(&oiv) },
                wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(&cov) },
                wgpu::BindGroupEntry { binding: 13, resource: wgpu::BindingResource::TextureView(&trv) },
            ],
        });
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3d outline"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &ov,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(&c.outline_pipeline);
        rp.set_bind_group(0, &bind, &[]);
        rp.set_vertex_buffer(0, quad.slice(..));
        rp.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
        rp.draw(0..3, 0..1);
    }
    // the shading (этап 7.2): the software engine's `shade_pass`, a shade level (0-15) per pixel
    let shade_tex = attach(c, w, h, wgpu::TextureFormat::R32Uint, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
    {
        let sv = shade_tex.create_view(&Default::default());
        let wv = word[cur].create_view(&Default::default());
        let dv = depth[cur].create_view(&Default::default());
        let cov = cover[cur].create_view(&Default::default());
        let trv = trans[cur].create_view(&Default::default());
        let bind = made_bind_group(device, &wgpu::BindGroupDescriptor {
            label: Some("3d shade"),
            layout: &c.bgl_shade,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&wv) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&dv) },
                wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(&cov) },
                wgpu::BindGroupEntry { binding: 13, resource: wgpu::BindingResource::TextureView(&trv) },
            ],
        });
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3d shade"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &sv,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(&c.shade_pipeline);
        rp.set_bind_group(0, &bind, &[]);
        rp.set_vertex_buffer(0, quad.slice(..));
        rp.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
        rp.draw(0..3, 0..1);
    }
    // the fog over the finished picture
    {
        let fv = fin.create_view(&Default::default());
        let wv = word[cur].create_view(&Default::default());
        let fgv = fog.create_view(&Default::default());
        let dv = depth[cur].create_view(&Default::default());
        let iv = id[cur].create_view(&Default::default());
        let ov = outline_tex.create_view(&Default::default());
        let sv = shade_tex.create_view(&Default::default());
        let fog_bind = made_bind_group(device, &wgpu::BindGroupDescriptor {
            label: Some("3d fog"),
            layout: &c.bgl_fog,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&wv) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&fgv) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&dv) },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&iv) },
                wgpu::BindGroupEntry { binding: 14, resource: wgpu::BindingResource::TextureView(&ov) },
                wgpu::BindGroupEntry { binding: 15, resource: wgpu::BindingResource::TextureView(&sv) },
            ],
        });
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3d fog"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &fv,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(&c.fog_pipeline);
        rp.set_bind_group(0, &fog_bind, &[]);
        rp.set_vertex_buffer(0, quad.slice(..));
        rp.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
        rp.draw(0..3, 0..1);
    }
    // the picture back: copy each attachment to a mappable buffer, then read the rows
    let (ow, oh) = (w / k, h / k);
    let res = (k > 1).then(|| {
        let res = attach(c, ow, oh, wgpu::TextureFormat::R32Uint, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
        let kb = frame_buffer(c, "3d resolve k", &[k as u32, 0, 0, 0].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>(), wgpu::BufferUsages::UNIFORM);
        let fiv = fin.create_view(&Default::default());
        let bind = made_bind_group(device, &wgpu::BindGroupDescriptor {
            label: Some("3d resolve"),
            layout: &c.bgl_resolve,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&fiv) },
                wgpu::BindGroupEntry { binding: 16, resource: kb.as_entire_binding() },
            ],
        });
        let rv = res.create_view(&Default::default());
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3d resolve"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &rv,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(&c.resolve_pipeline);
        rp.set_bind_group(0, &bind, &[]);
        rp.set_vertex_buffer(0, quad.slice(..));
        rp.set_viewport(0.0, 0.0, ow as f32, oh as f32, 0.0, 1.0);
        rp.draw(0..3, 0..1);
        drop(rp);
        res
    });
    let buf_word = match &res {
        Some(r) => readback_buf(c, &mut enc, r, ow as u32, oh as u32),
        None => readback_buf(c, &mut enc, &fin, w as u32, h as u32),
    };
    let aux = maps.then(|| {
        [
            readback_buf(c, &mut enc, &oid[cur], w as u32, h as u32),
            readback_buf(c, &mut enc, &cover[cur], w as u32, h as u32),
            readback_buf(c, &mut enc, &trans[cur], w as u32, h as u32),
            readback_buf(c, &mut enc, &depth[cur], w as u32, h as u32),
        ]
    });
    let t_enc = t0.elapsed();
    c.queue.submit([enc.finish()]);
    let timing = std::env::var_os("SUNPETAL_GPU_TIMING").is_some();
    if timing {
        let _ = c.device.poll(wgpu::PollType::wait_indefinitely()); // the GPU's work alone
    }
    let t_gpu = t0.elapsed();
    let words = readback_words(&c.device, &buf_word, ow, oh)?;
    if timing {
        // cumulative ms: textures decoded and uploaded, polygons built, resources made, commands
        // recorded, the GPU's work (waited for alone), then the readback
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
        eprintln!("gpu frame {w}x{h}: atlas {:.2}, build {:.2}, resources {:.2}, encode {:.2}, gpu {:.2}, readback done {:.2} ms", ms(t_atlas), ms(t_build), ms(t_res), ms(t_enc), ms(t_gpu), ms(t0.elapsed()));
        eprintln!("  serial passes: {n_blend} blend, {n_mask} mask (of {} polygons), {n_skipped} off-picture; {:.1}% of the picture copied per polygon on average", built.serial.len(), if n_blend + n_mask > 0 { px_copied as f64 / (built.serial.len() as f64 * (w * h) as f64) * 100.0 } else { 0.0 });
        let m = take_made();
        eprintln!("  made: {} textures ({:.1} MB; {} from the pool), {} buffers ({:.1} MB), {} bind groups; {:.2} MB written by the queue; atlas {}", m[0], m[1] as f64 / 1e6, m[6], m[2], m[3] as f64 / 1e6, m[4], m[5] as f64 / 1e6, if m[7] > 0 { "kept" } else { "made" });
    }
    let (mut oid_px, mut cover_px, mut trans_px, mut depth_px) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    if let Some([bo, bc, bt, bd]) = &aux {
        oid_px = readback_bytes(&c.device, bo, w, h)?;
        cover_px = readback_bytes(&c.device, bc, w, h)?;
        trans_px = readback_bytes(&c.device, bt, w, h)?;
        depth_px = readback_floats(&c.device, bd, w, h)?;
    }
    Ok(Rendered { words, oid: oid_px, cover: cover_px, trans: trans_px, depth: depth_px, notes })
}

type PoolKey = (u32, u32, wgpu::TextureFormat, wgpu::TextureUsages);

/// What the pool may hold: attachments of the sizes the last frames used (a change of the scale or the width
/// leaves the old sizes to be dropped first), at most this many bytes and entries (этап 9.1.3).
const POOL_MAX_BYTES: u64 = 192 << 20;
const POOL_MAX_ENTRIES: usize = 64;

fn key_bytes(k: &PoolKey) -> u64 {
    k.0 as u64 * k.1 as u64 * k.2.block_copy_size(None).unwrap_or(4) as u64
}

/// An attachment on loan from the pool: it goes back when the frame lets it go. It holds what the last frame
/// left in it, so a pass that reads it must have written all of it first (the passes clear or copy over).
struct Pooled {
    tex: Option<wgpu::Texture>,
    key: PoolKey,
    ctx: &'static Ctx,
}

impl std::ops::Deref for Pooled {
    type Target = wgpu::Texture;
    fn deref(&self) -> &wgpu::Texture {
        self.tex.as_ref().unwrap()
    }
}

impl Drop for Pooled {
    fn drop(&mut self) {
        let Some(tex) = self.tex.take() else { return };
        let mut pool = self.ctx.pool.lock().unwrap();
        pool.push((self.key, tex));
        let mut bytes: u64 = pool.iter().map(|(k, _)| key_bytes(k)).sum();
        // the oldest go first: they are the sizes the frames stopped asking for
        while pool.len() > POOL_MAX_ENTRIES || bytes > POOL_MAX_BYTES {
            let (k, _) = pool.remove(0);
            bytes -= key_bytes(&k);
        }
    }
}

type BufKey = (u64, wgpu::BufferUsages);

const BUF_POOL_MAX_ENTRIES: usize = 32;

/// A small buffer on loan from the pool, written with `queue.write_buffer` each frame instead of made anew.
struct PooledBuf {
    buf: Option<wgpu::Buffer>,
    key: BufKey,
    ctx: &'static Ctx,
}

impl std::ops::Deref for PooledBuf {
    type Target = wgpu::Buffer;
    fn deref(&self) -> &wgpu::Buffer {
        self.buf.as_ref().unwrap()
    }
}

impl Drop for PooledBuf {
    fn drop(&mut self) {
        let Some(buf) = self.buf.take() else { return };
        let mut pool = self.ctx.buf_pool.lock().unwrap();
        pool.push((self.key, buf));
        while pool.len() > BUF_POOL_MAX_ENTRIES {
            pool.remove(0);
        }
    }
}

/// A buffer of `contents`' size (a multiple of 4) holding them: from the pool and written by the queue, or new.
/// `usage` has COPY_DST added.
fn frame_buffer(c: &'static Ctx, label: &str, contents: &[u8], usage: wgpu::BufferUsages) -> PooledBuf {
    let usage = usage | wgpu::BufferUsages::COPY_DST;
    let key = (contents.len() as u64, usage);
    let mut pool = c.buf_pool.lock().unwrap();
    let taken = pool.iter().position(|(k, _)| *k == key).map(|i| pool.swap_remove(i).1);
    drop(pool);
    let buf = taken.unwrap_or_else(|| made_buffer(&c.device, &wgpu::BufferDescriptor { label: Some(label), size: key.0, usage, mapped_at_creation: false }));
    made(5, contents.len() as u64);
    c.queue.write_buffer(&buf, 0, contents);
    PooledBuf { buf: Some(buf), key, ctx: c }
}

/// A 2D texture of the picture's size for one of the passes' attachments: one the pool has of that kind,
/// or a new one.
fn attach(c: &'static Ctx, w: usize, h: usize, format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> Pooled {
    let key = (w as u32, h as u32, format, usage);
    let mut pool = c.pool.lock().unwrap();
    if let Some(i) = pool.iter().position(|(k, _)| *k == key) {
        let (_, tex) = pool.swap_remove(i);
        made(6, 1);
        return Pooled { tex: Some(tex), key, ctx: c };
    }
    drop(pool);
    let tex = made_texture(&c.device, &wgpu::TextureDescriptor {
        label: Some("3d attachment"),
        size: wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    Pooled { tex: Some(tex), key, ctx: c }
}

/// How many attachments the pool holds (for the tests).
#[cfg(test)]
fn pool_len() -> usize {
    ctx().map_or(0, |c| c.pool.lock().unwrap().len())
}

/// A mappable buffer a picture-sized attachment is copied into (rows padded to 256 bytes), to be
/// read back by `readback_words`/`readback_bytes` after the submit.
fn readback_buf(c: &'static Ctx, enc: &mut wgpu::CommandEncoder, tex: &wgpu::Texture, w: u32, h: u32) -> PooledBuf {
    let row = ((w * 4).div_ceil(256) * 256) as u64;
    let usage = wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST;
    let key = (row * h as u64, usage);
    let mut pool = c.buf_pool.lock().unwrap();
    let taken = pool.iter().position(|(k, _)| *k == key).map(|i| pool.swap_remove(i).1);
    drop(pool);
    // a read-back buffer comes back unmapped (`readback_*` unmap it) and is copied over whole
    let buf = taken.unwrap_or_else(|| made_buffer(&c.device, &wgpu::BufferDescriptor { label: Some("3d readback"), size: key.0, usage, mapped_at_creation: false }));
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row as u32), rows_per_image: Some(h) },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    PooledBuf { buf: Some(buf), key, ctx: c }
}

/// Reads a mapped readback buffer's words (`w` words a row, rows padded to 256 bytes).
fn readback_words(device: &wgpu::Device, buf: &wgpu::Buffer, w: usize, h: usize) -> Result<Vec<u32>, String> {
    let row = ((w as u32 * 4).div_ceil(256) * 256) as usize;
    let slice = buf.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let map = slice.get_mapped_range().map_err(|_| "the 3D picture could not be read back")?;
    let mut v = Vec::with_capacity(w * h);
    for y in 0..h {
        let base = y * row;
        for x in 0..w {
            let p = base + x * 4;
            v.push(u32::from_le_bytes([map[p], map[p + 1], map[p + 2], map[p + 3]]));
        }
    }
    drop(map);
    buf.unmap();
    Ok(v)
}

/// Reads a mapped readback buffer one byte a pixel (the 0-63-valued attachments).
fn readback_bytes(device: &wgpu::Device, buf: &wgpu::Buffer, w: usize, h: usize) -> Result<Vec<u8>, String> {
    let row = ((w as u32 * 4).div_ceil(256) * 256) as usize;
    let slice = buf.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let map = slice.get_mapped_range().map_err(|_| "the 3D picture could not be read back")?;
    let mut v = Vec::with_capacity(w * h);
    for y in 0..h {
        let base = y * row;
        for x in 0..w {
            v.push(map[base + x * 4]);
        }
    }
    drop(map);
    buf.unmap();
    Ok(v)
}

/// Reads a mapped readback buffer's floats (the depth attachment, `R32Float`).
fn readback_floats(device: &wgpu::Device, buf: &wgpu::Buffer, w: usize, h: usize) -> Result<Vec<f32>, String> {
    Ok(readback_words(device, buf, w, h)?.into_iter().map(f32::from_bits).collect())
}

/// The rear plane's bitmap as a 256x256 `Rgba32Uint` texture: each texel is the colour word
/// (RGB666 + alpha, from texture slot 2), the fog flag and the depth (slot 3's 15 bits as the
/// depth buffer holds them, bitcast to u32). `None` when the rear plane (DISP3DCNT bit 14) is off.
fn rear_data(job: &Job) -> Option<Vec<u8>> {
    if job.disp3dcnt & 0x4000 == 0 {
        return None;
    }
    let rd = |base: usize, x: usize, y: usize| -> u16 {
        let o = base + (y * 256 + x) * 2;
        u16::from_le_bytes([job.tex.get(o).copied().unwrap_or(0), job.tex.get(o + 1).copied().unwrap_or(0)])
    };
    let mut data = Vec::with_capacity(256 * 256 * 16);
    for y in 0..256 {
        for x in 0..256 {
            let c = rd(0x40000, x, y);
            let d = rd(0x60000, x, y);
            let (r, g, b) = render3d::rgb555(c & 0x7FFF);
            let word = r | g << 6 | b << 12 | if c & 0x8000 != 0 { 31 } else { 0 } << 18;
            data.extend_from_slice(&word.to_le_bytes());
            data.extend_from_slice(&((d & 0x8000 != 0) as u32).to_le_bytes());
            data.extend_from_slice(&render3d::clear_depth((d & 0x7FFF) as u16).to_bits().to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes());
        }
    }
    Some(data)
}

const SHADER: &str = r#"
struct Globals {
  w: u32,
  h: u32,
  clear_word: u32,
  wbuffer: u32,
  alpha_test: u32,
  alpha_ref: u32,
  highlight: u32,
  bilinear: u32,
  // the scale (the picture is `scale` times the DS's size each way)
  scale: u32,
  // the rear plane's bitmap origin (CLRIMAGE_OFFSET: x, then y << 8)
  rear_ox: u32,
  rear_oy: u32,
  // the fog: enabled (DISP3DCNT bit 7), alpha-only (bit 6), its colour and alpha as one word,
  // the step (0x400 >> shift, at least 1) and the table's offset
  fog_enable: u32,
  fog_alpha_only: u32,
  fog_color: u32,
  fog_step: u32,
  fog_offset: u32,
  // the clear plane's polygon ID (CLEAR_COLOR bits 24-29), and whether alpha blending is on
  // (DISP3DCNT bit 3: without it a translucent polygon draws opaque)
  clear_id: u32,
  blend_on: u32,
  // our outline (этап 7.1): its strength (0-1) and width in DS pixels (0.25-2)
  outline: f32,
  outline_width: f32,
  // our shading (этап 7.2): its strength (0-1), how many steps a ray takes, and the three rays
  // (computed on the CPU, exactly the software engine's)
  shadow: f32,
  shade_steps: u32,
  ray0x: f32,
  ray0y: f32,
  ray1x: f32,
  ray1y: f32,
  ray2x: f32,
  ray2y: f32,
  // the DS's edge marking (этап 7.3): on (DISP3DCNT bit 5) and the eight edge colours, two
  // BGR555 words packed per scalar
  edge_on: u32,
  edge0: u32,
  edge1: u32,
  edge2: u32,
  edge3: u32,
  // the widened view (этап 7.4): on (the width is past the DS's 4:3 and SUNPETAL_NO_EXTEND is
  // absent), the side's extent in pixels, the central columns, and the clear depth
  extend_on: u32,
  e: u32,
  inner: u32,
  cd: f32,
  // the toon table, four 15-bit colours per vec4 (uniform arrays need a stride of 16)
  toon: array<vec4u, 8>,
  // the fog table, four u8 densities per vec4
  fogt: array<vec4u, 8>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<storage, read> tris: array<Tri>;
@group(0) @binding(2) var atlas: texture_2d<u32>;
@group(0) @binding(3) var rear: texture_2d<u32>;
@group(0) @binding(4) var color: texture_2d<u32>;
@group(0) @binding(5) var fogtex: texture_2d<u32>;
@group(0) @binding(6) var depth_tex: texture_2d<f32>;
// the accumulated picture the translucent pass blends over: the word, the polygon ID, the shadow
// mask and the depth (ping-ponged between polygons, so each reads the last one's result)
@group(0) @binding(7) var word_cur: texture_2d<u32>;
@group(0) @binding(8) var id_cur: texture_2d<u32>;
@group(0) @binding(9) var stencil_cur: texture_2d<u32>;
@group(0) @binding(10) var depth_cur: texture_2d<f32>;
// the accumulated auxiliary maps (этап 7.0): the last opaque polygon's ID, how much the
// translucent polygons over it hide the picture, and whether the depth is a translucent polygon's
@group(0) @binding(11) var oid_cur: texture_2d<u32>;
@group(0) @binding(12) var cover_cur: texture_2d<u32>;
@group(0) @binding(13) var trans_cur: texture_2d<u32>;
// the outline's line level (этап 7.1), the fog pass packs it into the word's bits 24-27
@group(0) @binding(14) var outline_tex: texture_2d<u32>;
// the shading's shade level (этап 7.2), the fog pass packs it into the word's bits 28-31
@group(0) @binding(15) var shade_tex: texture_2d<u32>;
// the widened view's row sources (этап 7.4), two texels a row (left, right): r/g the row's sky and
// far columns (bit 15 set when found), b the nearest row with a sky column (bit 15 when found,
// bits 0-11 the row), a that row's sky column
@group(0) @binding(16) var srcinfo_tex: texture_2d<u32>;
// the tiled translucent pass (этап 9.4): every translucent and shadow polygon's triangles in one
// buffer (`tris`), a table of the polygons (first triangle, triangle count, 1 for a shadow mask), the
// 16x16 tiles' lists of polygon indices in the job's order (offsets, then the indices), and the fog
// flags before the pass
@group(0) @binding(17) var<storage, read> polys: array<vec4u>;
@group(0) @binding(18) var<storage, read> tile_off: array<u32>;
@group(0) @binding(19) var<storage, read> tile_idx: array<u32>;
@group(0) @binding(20) var fog_old: texture_2d<u32>;

// One fan triangle: the exact vertex positions (the software engine's), its area, the ownership
// of its edges (bits 0-2), the mode, the interpolation data of its three vertices, and its
// texture's place in the atlas.
struct Tri {
  ax: f32, ay: f32, bx: f32, by: f32, cx: f32, cy: f32,
  area: f32,
  owns: u32,
  mode: u32,
  iw0: f32, d0: f32, r0: f32, g0: f32, b0: f32, s0: f32, t0: f32,
  iw1: f32, d1: f32, r1: f32, g1: f32, b1: f32, s1: f32, t1: f32,
  iw2: f32, d2: f32, r2: f32, g2: f32, b2: f32, s2: f32, t2: f32,
  // the picture's corner in the atlas and its size in texels (0: no texture)
  ux: u32, uy: u32, tw: u32, th: u32,
  // the original texture's size in texels, 0 for the DS's own texture (its coordinates already
  // are texels); and the flags: bit 0/1 S/T repeat, bit 2/3 S/T flip, bit 4 a replacement
  // picture (its coordinates scale)
  ow: f32, oh: f32,
  tflags: u32,
  // the polygon's fog bit (POLYGON_ATTR bit 15)
  fog: u32,
  // the polygon's alpha (POLYGON_ATTR bits 16-20), its ID (bits 24-29), the equal-depth test
  // (bit 14) and whether a translucent polygon writes the depth (bit 11)
  polyalpha: u32,
  pid: u32,
  eq_depth: u32,
  write_trans: u32,
};

struct VIn {
  @location(0) pos: vec2f,
};

struct VOut {
  @builtin(position) clip: vec4f,
  @location(0) @interpolate(flat) tri_i: u32,
};

@vertex
fn vs(in: VIn, @builtin(vertex_index) vi: u32) -> VOut {
  var o: VOut;
  o.clip = vec4f(in.pos.x / f32(g.w) * 2.0 - 1.0, 1.0 - in.pos.y / f32(g.h) * 2.0, 0.0, 1.0);
  o.tri_i = vi / 3u;
  return o;
}

struct FOut {
  @location(0) word: vec4u,
  @location(1) fog: vec4u,
  @location(2) depth_out: vec4f,
  @location(3) id: vec4u,
  @location(4) stencil: vec4u,
  @location(5) oid: vec4u,
  @location(6) cover: vec4u,
  @location(7) trans: vec4u,
  @builtin(frag_depth) depth: f32,
};

// the translucent pass's output: the same eight attachments, the depth test done in the shader
struct FTrans {
  @location(0) word: vec4u,
  @location(1) fog: vec4u,
  @location(2) depth_out: vec4f,
  @location(3) id: vec4u,
  @location(4) stencil: vec4u,
  @location(5) oid: vec4u,
  @location(6) cover: vec4u,
  @location(7) trans: vec4u,
};

// the shadow mask's output: just the mask, one pixel set where it lies behind something
struct FMask {
  @location(0) stencil: vec4u,
};

fn edge(p: vec2f, q: vec2f, t: vec2f) -> f32 {
  // each product rounded on its own, like the software engine's: a compiler fusing `a*b - c*d`
  // into one fma would shift the difference by ulps where the products cancel (the parity tests
  // caught a 4-ulp shift right there)
  let a = fma(q.x - p.x, t.y - p.y, 0.0);
  let b = fma(q.y - p.y, t.x - p.x, 0.0);
  return a - b;
}

// the software engine's vertex colour to RGB666: (c * 63 / 31) rounded, clamped
fn to6(c: f32) -> u32 {
  let v = round(div(c * 63.0, 31.0));
  return u32(clamp(v, 0.0, 63.0));
}

// a correctly rounded f32 division: GPUs divide only within a few ulps, the software engine's
// divisions round correctly, and an ulp the wrong way flips a texel at its boundary (the parity
// tests caught exactly that on the diagonal). Two Newton steps on the quotient make the result
// correct: each remainder is exact (`fma`), and the corrections' errors are second-order, so
// after the second step the answer is within a fraction of an ulp.
fn div(p: f32, d: f32) -> f32 {
  let q0 = p / d;
  let r0 = fma(-q0, d, p);
  let q1 = q0 + r0 / d;
  let r1 = fma(-q1, d, p);
  return q1 + r1 / d;
}

// a 5-bit colour channel to 6 bits, as the software engine widens it
fn c5to6(c: u32) -> u32 {
  return select(0u, c * 2u + 1u, c != 0u);
}

// a texture coordinate to its texel index, as the software engine wraps it: repeated (mirrored
// when the flip flag is set), or clamped to the edges
fn wrapi(v: i32, size: i32, repeat: bool, flip: bool) -> i32 {
  if (!repeat) {
    return clamp(v, 0, size - 1);
  }
  var q = v / size;
  var m = v - q * size;
  if (m < 0) {
    q -= 1;
    m += size;
  }
  if (flip && (q & 1) != 0) {
    m = size - 1 - m;
  }
  return m;
}

// one atlas texel of a wrapped texture coordinate
fn tap(t: Tri, x: i32, y: i32) -> vec4u {
  let xi = wrapi(x, i32(t.tw), (t.tflags & 1u) != 0u, (t.tflags & 4u) != 0u);
  let yi = wrapi(y, i32(t.th), (t.tflags & 2u) != 0u, (t.tflags & 8u) != 0u);
  return textureLoad(atlas, vec2u(t.ux + u32(xi), t.uy + u32(yi)), 0u);
}

// the texture coordinate in the picture's texels: a replacement's picture is sampled over the
// original texture's coordinates, so the coordinate divides by the original's size
fn texel_coord(s: f32, ow: f32, size: u32, scaled: bool) -> f32 {
  if (scaled) {
    return div(s, ow) * f32(size);
  }
  return s;
}

// one texel, nearest, as the software engine reads it: (r, g, b) in 6 bits, alpha in 5 (the
// atlas holds the decode's bytes, the colour 6-bit widened and the alpha a 5-bit value to 255)
fn tex_nearest(t: Tri, s: f32, tt: f32) -> vec4u {
  let su = texel_coord(s, t.ow, t.tw, (t.tflags & 16u) != 0u);
  let tv = texel_coord(tt, t.oh, t.th, (t.tflags & 16u) != 0u);
  let c = tap(t, i32(floor(su)), i32(floor(tv)));
  return vec4u(c.x >> 2u, c.y >> 2u, c.z >> 2u, (c.w * 31u + 127u) / 255u);
}

// the software engine's `bilinear`: four texels blended by the fractional position, weighted by
// alpha so a transparent texel's colour never bleeds into its neighbours; the result is already
// (r, g, b) in 6 bits, alpha in 5, like `tex_nearest`'s
fn tex_bilinear(t: Tri, s: f32, tt: f32) -> vec4u {
  let su = texel_coord(s, t.ow, t.tw, (t.tflags & 16u) != 0u) - 0.5;
  let tv = texel_coord(tt, t.oh, t.th, (t.tflags & 16u) != 0u) - 0.5;
  let x0 = i32(floor(su));
  let y0 = i32(floor(tv));
  let fx = su - f32(x0);
  let fy = tv - f32(y0);
  let ws = vec4f((1.0 - fx) * (1.0 - fy), fx * (1.0 - fy), (1.0 - fx) * fy, fx * fy);
  var r = 0.0;
  var g = 0.0;
  var b = 0.0;
  var a = 0.0;
  var wa = 0.0;
  for (var i = 0u; i < 4u; i++) {
    let c = tap(t, x0 + i32(i & 1u), y0 + i32(i >> 1u));
    let k = ws[i] * f32((c.w * 31u + 127u) / 255u);
    r = r + fma(k, f32(c.x >> 2u), 0.0);
    g = g + fma(k, f32(c.y >> 2u), 0.0);
    b = b + fma(k, f32(c.z >> 2u), 0.0);
    a = a + k;
    wa = wa + ws[i];
  }
  return vec4u(u32(min(floor(div(r, a) + 0.5), 63.0)), u32(min(floor(div(g, a) + 0.5), 63.0)), u32(min(floor(div(b, a) + 0.5), 63.0)), u32(min(floor(div(a, wa) + 0.5), 31.0)));
}

// the tiled pass's version of `bary`: the weights and whether the pixel is inside, no discard. The drawing
// passes keep `bary` as it was: rewriting it through this changed the compiled arithmetic (the parity tests
// caught it)
struct Bary {
  l: vec3f,
  ok: bool,
};

fn bary_try(t: Tri, pt: vec2f) -> Bary {
  let a = vec2f(t.ax, t.ay);
  let b = vec2f(t.bx, t.by);
  let c = vec2f(t.cx, t.cy);
  let w0 = edge(b, c, pt);
  let w1 = edge(c, a, pt);
  let w2 = edge(a, b, pt);
  let in0 = w0 > 0.0 || (w0 == 0.0 && (t.owns & 1u) != 0u);
  let in1 = w1 > 0.0 || (w1 == 0.0 && (t.owns & 2u) != 0u);
  let in2 = w2 > 0.0 || (w2 == 0.0 && (t.owns & 4u) != 0u);
  var o: Bary;
  o.ok = in0 && in1 && in2;
  o.l = vec3f(div(w0, t.area), div(w1, t.area), div(w2, t.area));
  return o;
}

// the pixel's barycentric weights in a fan triangle, or it is discarded outside (the exact
// inside test of the software engine: strictly in, or on an edge the polygon owns)
fn bary(t: Tri, pt: vec2f) -> vec3f {
  let a = vec2f(t.ax, t.ay);
  let b = vec2f(t.bx, t.by);
  let c = vec2f(t.cx, t.cy);
  let w0 = edge(b, c, pt);
  let w1 = edge(c, a, pt);
  let w2 = edge(a, b, pt);
  let in0 = w0 > 0.0 || (w0 == 0.0 && (t.owns & 1u) != 0u);
  let in1 = w1 > 0.0 || (w1 == 0.0 && (t.owns & 2u) != 0u);
  let in2 = w2 > 0.0 || (w2 == 0.0 && (t.owns & 4u) != 0u);
  if (!(in0 && in1 && in2)) {
    discard;
  }
  return vec3f(div(w0, t.area), div(w1, t.area), div(w2, t.area));
}

// the depth at a pixel (its barycentric weights in l), exactly as the software engine has it
fn depth_at(t: Tri, l: vec3f) -> f32 {
  if (g.wbuffer != 0u) {
    let den = (fma(l.x, t.iw0, 0.0) + fma(l.y, t.iw1, 0.0)) + fma(l.z, t.iw2, 0.0);
    return div(div(1.0, den), 16777215.0);
  }
  return (fma(l.x, t.d0, 0.0) + fma(l.y, t.d1, 0.0)) + fma(l.z, t.d2, 0.0);
}

// the shaded colour and alpha of a pixel: (r6, g6, b6, oa); a transparent texel is gone (alpha 0 comes
// back), the alpha test may take more, and without blending a translucent result turns opaque
fn shade_try(t: Tri, l: vec3f) -> vec4u {
  // the colour, perspective-correct, exactly as the software engine weights it (the sums keep
  // the engine's order of operations)
  let p0 = l.x * t.iw0;
  let p1 = l.y * t.iw1;
  let p2 = l.z * t.iw2;
  let den = (p0 + p1) + p2;
  let q0 = div(p0, den);
  let q1 = div(p1, den);
  let q2 = div(p2, den);
  let vr = (fma(q0, t.r0, 0.0) + fma(q1, t.r1, 0.0)) + fma(q2, t.r2, 0.0);
  let vg = (fma(q0, t.g0, 0.0) + fma(q1, t.g1, 0.0)) + fma(q2, t.g2, 0.0);
  let vb = (fma(q0, t.b0, 0.0) + fma(q1, t.b1, 0.0)) + fma(q2, t.b2, 0.0);
  // the texture coordinates, the same weights (the software engine interpolates them like the
  // colour)
  let s_ = (fma(q0, t.s0, 0.0) + fma(q1, t.s1, 0.0)) + fma(q2, t.s2, 0.0);
  let tt = (fma(q0, t.t0, 0.0) + fma(q1, t.t1, 0.0)) + fma(q2, t.t2, 0.0);
  var r6 = to6(vr);
  var g6 = to6(vg);
  var b6 = to6(vb);
  var toon = vec3u(0u);
  if (t.mode == 2u) {
    // toon (and highlight): the vertex colour's red picks the table's colour
    let ri = min(r6 >> 1u, 31u);
    let tv = g.toon[ri >> 2u];
    let tc = tv[ri & 3u];
    toon = vec3u(c5to6(tc & 31u), c5to6(tc >> 5u & 31u), c5to6(tc >> 10u & 31u));
    if (g.highlight != 0u) {
      // the highlight: the vertex colour turns gray; the table's colour is added after the texture
      let gray = r6;
      r6 = gray;
      g6 = gray;
      b6 = gray;
    } else {
      r6 = toon.x;
      g6 = toon.y;
      b6 = toon.z;
    }
  }
  var oa: u32 = t.polyalpha;
  if (t.tw != 0u) {
    let c = select(tex_nearest(t, s_, tt), tex_bilinear(t, s_, tt), g.bilinear != 0u);
    let tr6 = c.x;
    let tg6 = c.y;
    let tb6 = c.z;
    let ta5 = c.w;
    if (t.mode == 1u) {
      // decal: the texture over the vertex colour by its alpha (the polygon's own alpha stays)
      if (ta5 == 0u) {
        // the vertex colour
      } else if (ta5 == 31u) {
        r6 = tr6;
        g6 = tg6;
        b6 = tb6;
      } else {
        r6 = (tr6 * ta5 + r6 * (31u - ta5)) / 31u;
        g6 = (tg6 * ta5 + g6 * (31u - ta5)) / 31u;
        b6 = (tb6 * ta5 + b6 * (31u - ta5)) / 31u;
      }
    } else {
      // modulate (and the toon): the colours and the alpha multiply
      r6 = ((tr6 + 1u) * (r6 + 1u) - 1u) / 64u;
      g6 = ((tg6 + 1u) * (g6 + 1u) - 1u) / 64u;
      b6 = ((tb6 + 1u) * (b6 + 1u) - 1u) / 64u;
      oa = ((ta5 + 1u) * (t.polyalpha + 1u) - 1u) / 32u;
    }
  }
  if (t.mode == 2u && g.highlight != 0u) {
    // the table's colour over the textured picture
    r6 = min(r6 + toon.x, 63u);
    g6 = min(g6 + toon.y, 63u);
    b6 = min(b6 + toon.z, 63u);
  }
  // without blending a translucent result draws opaque
  if (g.blend_on == 0u && oa > 0u) {
    oa = 31u;
  }
  // a transparent texel is gone; the alpha test may take more
  if (oa == 0u || (g.alpha_test != 0u && oa <= g.alpha_ref)) {
    return vec4u(0u);
  }
  return vec4u(r6, g6, b6, oa);
}

// the same, the pixel discarded where it is gone
fn shade(t: Tri, l: vec3f) -> vec4u {
  let c = shade_try(t, l);
  if (c.w == 0u) {
    discard;
  }
  return c;
}

@fragment
fn fs(in: VOut) -> FOut {
  let t = tris[in.tri_i];
  // the pixel centre, exactly as the software engine computes it (x + 0.5, y + 0.5): the
  // rasterizer's interpolated position may be an ulp off, which the exact engine never is
  let pt = vec2f(floor(in.clip.x) + 0.5, floor(in.clip.y) + 0.5);
  let l = bary(t, pt);
  let depth = depth_at(t, l);
  let c = shade(t, l);
  var o: FOut;
  o.word = vec4u(c.x | c.y << 6u | c.z << 12u | 31u << 18u, 0u, 0u, 0u);
  o.fog = vec4u(t.fog, 0u, 0u, 0u);
  o.depth_out = vec4f(depth, 0.0, 0.0, 0.0);
  o.id = vec4u(t.pid, 0u, 0u, 0u);
  o.stencil = vec4u(0u, 0u, 0u, 0u);
  o.oid = vec4u(t.pid, 0u, 0u, 0u);
  o.cover = vec4u(0u, 0u, 0u, 0u);
  o.trans = vec4u(0u, 0u, 0u, 0u);
  o.depth = depth;
  return o;
}

// a translucent polygon, drawn in its own pass in the order the job holds them: the depth test in
// the shader against the accumulated depth, the shadow mask's mark, then the DS blend
// (src*(a+1) + dst*(31-a))/32 over what is there (an alpha-0 pixel is just written)
@fragment
fn fs_trans(in: VOut) -> FTrans {
  let t = tris[in.tri_i];
  let pix = vec2u(u32(floor(in.clip.x)), u32(floor(in.clip.y)));
  let pt = vec2f(floor(in.clip.x) + 0.5, floor(in.clip.y) + 0.5);
  let l = bary(t, pt);
  let depth = depth_at(t, l);
  let old = textureLoad(word_cur, pix, 0u).x;
  let old_id = textureLoad(id_cur, pix, 0u).x;
  let stencil = textureLoad(stencil_cur, pix, 0u).x;
  let old_depth = textureLoad(depth_cur, pix, 0u).x;
  let old_oid = textureLoad(oid_cur, pix, 0u).x;
  let old_cover = textureLoad(cover_cur, pix, 0u).x;
  let old_trans = textureLoad(trans_cur, pix, 0u).x;
  let old_alpha = (old >> 18u) & 31u;
  // the depth test (the software engine's tolerance for the equal test)
  let tol = 0x200.0 / 16777215.0;
  let behind = select(depth >= old_depth, abs(depth - old_depth) > tol, t.eq_depth != 0u);
  if (behind) {
    discard;
  }
  if (t.mode == 3u) {
    // a shadow: only where a mask lay behind, and not on the polygon's own ID
    if (stencil == 0u || old_id == t.pid) {
      discard;
    }
  }
  let c = shade(t, l);
  let oa = c.w;
  if (oa == 31u) {
    // an opaque result (blending off): overwrite, write the depth
    var o: FTrans;
    o.word = vec4u(c.x | c.y << 6u | c.z << 12u | 31u << 18u, 0u, 0u, 0u);
    o.fog = vec4u(t.fog, 0u, 0u, 0u);
    o.depth_out = vec4f(depth, 0.0, 0.0, 0.0);
    o.id = vec4u(t.pid, 0u, 0u, 0u);
    o.stencil = vec4u(0u, 0u, 0u, 0u);
    o.oid = vec4u(t.pid, 0u, 0u, 0u);
    o.cover = vec4u(0u, 0u, 0u, 0u);
    o.trans = vec4u(0u, 0u, 0u, 0u);
    return o;
  }
  // a translucent polygon does not draw over its own ID
  if (old_id == t.pid && old_alpha > 0u && old_alpha < 31u) {
    discard;
  }
  var word: u32;
  if (old_alpha == 0u) {
    word = c.x | c.y << 6u | c.z << 12u | oa << 18u;
  } else {
    let nr = dsmix(c.x, old & 63u, oa);
    let ng = dsmix(c.y, old >> 6u & 63u, oa);
    let nb = dsmix(c.z, old >> 12u & 63u, oa);
    let na = max(old_alpha, oa);
    word = nr | ng << 6u | nb << 12u | na << 18u;
  }
  var o: FTrans;
  o.word = vec4u(word, 0u, 0u, 0u);
  o.fog = vec4u(t.fog, 0u, 0u, 0u);
  o.depth_out = vec4f(select(old_depth, depth, t.write_trans != 0u), 0.0, 0.0, 0.0);
  o.id = vec4u(t.pid, 0u, 0u, 0u);
  o.stencil = vec4u(0u, 0u, 0u, 0u);
  // a translucent polygon leaves the last opaque ID; it hides the picture by its alpha over what
  // is already hidden, and marks the depth as translucent when it writes it (bit 11)
  o.oid = vec4u(old_oid, 0u, 0u, 0u);
  o.cover = vec4u(select(old_cover, max(old_cover, oa), old_alpha > 0u), 0u, 0u, 0u);
  o.trans = vec4u(old_trans | t.write_trans, 0u, 0u, 0u);
  return o;
}

// the shadow mask (a mode-3 polygon of ID 0): no picture, a mark where it lies behind something
@fragment
fn fs_mask(in: VOut) -> FMask {
  let t = tris[in.tri_i];
  let pix = vec2u(u32(floor(in.clip.x)), u32(floor(in.clip.y)));
  let pt = vec2f(floor(in.clip.x) + 0.5, floor(in.clip.y) + 0.5);
  let l = bary(t, pt);
  let depth = depth_at(t, l);
  let old_depth = textureLoad(depth_cur, pix, 0u).x;
  let tol = 0x200.0 / 16777215.0;
  let behind = select(depth >= old_depth, abs(depth - old_depth) > tol, t.eq_depth != 0u);
  var o: FMask;
  if (behind) {
    o.stencil = vec4u(1u, 0u, 0u, 0u);
  } else {
    discard;
  }
  return o;
}

// the tiled translucent pass: the pixel takes its tile's polygons in the job's order, each fan triangle
// as `fs_trans`/`fs_mask` do it, over the state it keeps in locals; a pixel nothing changed is
// discarded, so the other ping-pong buffer keeps the picture it holds
@fragment
fn fs_tiled(in: VFull) -> FTrans {
  let pix = vec2u(u32(floor(in.clip.x)), u32(floor(in.clip.y)));
  let pt = vec2f(floor(in.clip.x) + 0.5, floor(in.clip.y) + 0.5);
  let tile = (pix.y / 16u) * ((g.w + 15u) / 16u) + pix.x / 16u;
  var word = textureLoad(word_cur, pix, 0u).x;
  var id = textureLoad(id_cur, pix, 0u).x;
  var stencil = textureLoad(stencil_cur, pix, 0u).x;
  var depth = textureLoad(depth_cur, pix, 0u).x;
  var oid = textureLoad(oid_cur, pix, 0u).x;
  var cover = textureLoad(cover_cur, pix, 0u).x;
  var trans = textureLoad(trans_cur, pix, 0u).x;
  var fogf = textureLoad(fog_old, pix, 0u).x;
  var changed = false;
  let tol = 0x200.0 / 16777215.0;
  for (var k = tile_off[tile]; k < tile_off[tile + 1u]; k++) {
    let pp = polys[tile_idx[k]];
    for (var ti = pp.x; ti < pp.x + pp.y; ti++) {
      let t = tris[ti];
      let b = bary_try(t, pt);
      if (!b.ok) {
        continue;
      }
      let dz = depth_at(t, b.l);
      let behind = select(dz >= depth, abs(dz - depth) > tol, t.eq_depth != 0u);
      if (pp.z == 1u) {
        // a shadow mask: a mark where it lies behind something
        if (behind) {
          stencil = 1u;
          changed = true;
        }
        continue;
      }
      if (behind) {
        continue;
      }
      if (t.mode == 3u) {
        if (stencil == 0u || id == t.pid) {
          continue;
        }
      }
      let c = shade_try(t, b.l);
      let oa = c.w;
      if (oa == 0u) {
        continue;
      }
      if (oa == 31u) {
        // an opaque result (blending off): overwrite, write the depth
        word = c.x | c.y << 6u | c.z << 12u | 31u << 18u;
        fogf = t.fog;
        depth = dz;
        id = t.pid;
        stencil = 0u;
        oid = t.pid;
        cover = 0u;
        trans = 0u;
        changed = true;
        continue;
      }
      let old_alpha = (word >> 18u) & 31u;
      // a translucent polygon does not draw over its own ID
      if (id == t.pid && old_alpha > 0u && old_alpha < 31u) {
        continue;
      }
      if (old_alpha == 0u) {
        word = c.x | c.y << 6u | c.z << 12u | oa << 18u;
      } else {
        let nr = dsmix(c.x, word & 63u, oa);
        let ng = dsmix(c.y, word >> 6u & 63u, oa);
        let nb = dsmix(c.z, word >> 12u & 63u, oa);
        let na = max(old_alpha, oa);
        word = nr | ng << 6u | nb << 12u | na << 18u;
      }
      fogf = t.fog;
      depth = select(depth, dz, t.write_trans != 0u);
      id = t.pid;
      stencil = 0u;
      cover = select(cover, max(cover, oa), old_alpha > 0u);
      trans = trans | t.write_trans;
      changed = true;
    }
  }
  if (!changed) {
    discard;
  }
  var o: FTrans;
  o.word = vec4u(word, 0u, 0u, 0u);
  o.fog = vec4u(fogf, 0u, 0u, 0u);
  o.depth_out = vec4f(depth, 0.0, 0.0, 0.0);
  o.id = vec4u(id, 0u, 0u, 0u);
  o.stencil = vec4u(stencil, 0u, 0u, 0u);
  o.oid = vec4u(oid, 0u, 0u, 0u);
  o.cover = vec4u(cover, 0u, 0u, 0u);
  o.trans = vec4u(trans, 0u, 0u, 0u);
  return o;
}

// one density of the fog table (four u8 packed into each vec4 component)
fn fogt_at(i: u32) -> u32 {
  let v = g.fogt[i >> 2u];
  return v[i & 3u];
}

// the fog density (0-128) at a 24-bit depth, as the software engine steps its table: the 32
// entries sit at FOG_OFFSET + (0x400 >> shift) times 1 to 32, linear between, the edge entries
// held past their ends (a density of 127 rounds up to 128)
fn fog_density(depth24: u32) -> u32 {
  let z = i32(depth24 >> 9u);
  let step = i32(g.fog_step);
  let pos = z - i32(g.fog_offset) - step;
  var d: i32;
  if (pos <= 0) {
    d = i32(fogt_at(0u)) * 256;
  } else {
    let i = pos / step;
    let f = pos % step;
    if (i >= 31) {
      d = i32(fogt_at(31u)) * 256;
    } else {
      let a = i32(fogt_at(u32(i)));
      let b = i32(fogt_at(u32(i) + 1u));
      d = a * 256 + (b - a) * 256 * f / step;
    }
  }
  let dd = (u32(d) / 256u) & 127u;
  return select(dd, 128u, dd >= 127u);
}

// a fog mix: `a` towards `b` by the density `d` (0-128)
fn fmix(a: u32, b: u32, d: u32) -> u32 {
  return (a * (128u - d) + b * d) >> 7u;
}

// the DS translucent blend: `s` towards `dd` by the alpha, (a+1)/32 to (31-a)/32
fn dsmix(s: u32, dd: u32, a: u32) -> u32 {
  return (s * (a + 1u) + dd * (31u - a)) / 32u;
}

struct VFull {
  @builtin(position) clip: vec4f,
};

@vertex
fn vs_full(in: VIn) -> VFull {
  var o: VFull;
  o.clip = vec4f(in.pos.x / f32(g.w) * 2.0 - 1.0, 1.0 - in.pos.y / f32(g.h) * 2.0, 0.0, 1.0);
  return o;
}

// the rear plane's bitmap (a full-screen quad drawn before the polygons, so it is the base the
// polygons draw over): each texel of the 256x256 texture holds the colour word, the fog flag and
// the depth (bitcast to u32) of the bitmap's pixel at CLRIMAGE_OFFSET + the picture's position,
// wrapped
@fragment
fn fs_rear(in: VFull) -> FOut {
  let x = u32(floor(in.clip.x));
  let y = u32(floor(in.clip.y));
  let sx = (x / g.scale + g.rear_ox) & 255u;
  let sy = (y / g.scale + g.rear_oy) & 255u;
  let e = textureLoad(rear, vec2u(sx, sy), 0u);
  let d = bitcast<f32>(e.z);
  var o: FOut;
  o.word = vec4u(e.x, 0u, 0u, 0u);
  o.fog = vec4u(e.y, 0u, 0u, 0u);
  o.depth_out = vec4f(d, 0.0, 0.0, 0.0);
  o.id = vec4u(g.clear_id, 0u, 0u, 0u);
  o.stencil = vec4u(0u, 0u, 0u, 0u);
  o.oid = vec4u(g.clear_id, 0u, 0u, 0u);
  o.cover = vec4u(0u, 0u, 0u, 0u);
  o.trans = vec4u(0u, 0u, 0u, 0u);
  o.depth = d;
  return o;
}

// the supersampling resolve: the pixel's k x k samples of the big picture averaged, colour
// weighted by alpha (a sample with no 3D has none), alpha as the share covered, the outline and
// shade levels over the covered samples; 1:1 with the software engine's `resolve`
@group(0) @binding(16) var<uniform> rk: vec4u;

@fragment
fn fs_resolve(in: VFull) -> FFog {
  let x = u32(floor(in.clip.x));
  let y = u32(floor(in.clip.y));
  let k = rk.x;
  var sa = 0u; var sr = 0u; var sg = 0u; var sb = 0u; var sl = 0u; var sh = 0u;
  for (var dy = 0u; dy < k; dy++) {
    for (var dx = 0u; dx < k; dx++) {
      let p = textureLoad(color, vec2u(x * k + dx, y * k + dy), 0u).x;
      let a = (p >> 18u) & 31u;
      if (a == 0u) { continue; }
      sa += a;
      sr += (p & 63u) * a;
      sg += ((p >> 6u) & 63u) * a;
      sb += ((p >> 12u) & 63u) * a;
      sl += ((p >> 24u) & 15u) * a;
      sh += ((p >> 28u) & 15u) * a;
    }
  }
  var o: FFog;
  o.word = vec4u(0u, 0u, 0u, 0u);
  let n = k * k;
  if (sa == 0u) { return o; }
  let alpha = (sa + n / 2u) / n;
  if (alpha == 0u) { return o; }
  let hs = sa / 2u;
  o.word = vec4u(((sr + hs) / sa) | (((sg + hs) / sa) << 6u) | (((sb + hs) / sa) << 12u) | (alpha << 18u) | (((sl + hs) / sa) << 24u) | (((sh + hs) / sa) << 28u), 0u, 0u, 0u);
  return o;
}

// the fog over the finished picture (a full-screen quad): a pixel that takes the fog moves
// towards the fog colour (unless the mode is alpha-only) and its alpha by the density at its
// depth; an alpha-0 pixel is no pixel at all (the 2D layers under it show). The outline's line
// level goes into the word's bits 24-27.
struct FFog {
  @location(0) word: vec4u,
};

// the outline's constants (этап 7.1), the software engine's: a jump in depth this big is a line,
// the interface at the near plane gets none, this much translucent cover hides the line, the
// silhouette against the 2D layers is this strong
const OUTLINE_STEP: f32 = 0.002;
const INTERFACE_DEPTH: f32 = 0.05;
const HIDING_ALPHA: u32 = 12u;
const SILHOUETTE: f32 = 0.5;

// rounding half away from zero (the software engine's `f32::round`): WGSL's own `round` ties to
// even, which flips a value of exactly 2.5 the wrong way
fn round_away(x: f32) -> f32 {
  return select(floor(x + 0.5), ceil(x - 0.5), x < 0.0);
}

// a neighbour at `q` takes the pair out of the outline (a translucent-depth or an interface pixel)
fn outline_pair_skip(q: vec2u) -> bool {
  return textureLoad(trans_cur, q, 0u).x != 0u || textureLoad(depth_tex, q, 0u).x < INTERFACE_DEPTH;
}

// the outline's line level (0-15) at a pixel, exactly the software engine's `outline_pass`: the
// near side of a jump in depth or of another (last opaque) polygon's ID gets a line, the border
// with the empty 2D a lighter one; the translucent cover weakens it, none from HIDING_ALPHA
fn outline_level(p: vec2u) -> u32 {
  let k = max(u32(round_away(g.outline_width * f32(g.scale))), 1u);
  if (p.x < k || p.y < k || p.x >= g.w - k || p.y >= g.h - k) {
    return 0u;
  }
  let alpha = (textureLoad(color, p, 0u).x >> 18u) & 31u;
  let d = textureLoad(depth_tex, p, 0u).x;
  if (alpha < 31u || textureLoad(trans_cur, p, 0u).x != 0u || d < INTERFACE_DEPTH) {
    return 0u;
  }
  let id = textureLoad(oid_cur, p, 0u).x;
  let cover = textureLoad(cover_cur, p, 0u).x;
  var edge = false;
  var silhouette = false;
  // the horizontal pair
  let al = vec2u(p.x - k, p.y);
  let ar = vec2u(p.x + k, p.y);
  if (!(outline_pair_skip(al) || outline_pair_skip(ar))) {
    let aal = (textureLoad(color, al, 0u).x >> 18u) & 31u;
    let aar = (textureLoad(color, ar, 0u).x >> 18u) & 31u;
    if (aal == 0u || aar == 0u) {
      silhouette = true;
    } else {
      let dal = textureLoad(depth_tex, al, 0u).x;
      let dar = textureLoad(depth_tex, ar, 0u).x;
      let ial = textureLoad(oid_cur, al, 0u).x;
      let iar = textureLoad(oid_cur, ar, 0u).x;
      if (dal + dar - 2.0 * d > OUTLINE_STEP * 2.0 || (ial != id && dal > d) || (iar != id && dar > d)) {
        edge = true;
      }
    }
  }
  // the vertical pair
  let au = vec2u(p.x, p.y - k);
  let ad = vec2u(p.x, p.y + k);
  if (!(outline_pair_skip(au) || outline_pair_skip(ad))) {
    let aau = (textureLoad(color, au, 0u).x >> 18u) & 31u;
    let aad = (textureLoad(color, ad, 0u).x >> 18u) & 31u;
    if (aau == 0u || aad == 0u) {
      silhouette = true;
    } else {
      let dau = textureLoad(depth_tex, au, 0u).x;
      let dad = textureLoad(depth_tex, ad, 0u).x;
      let iau = textureLoad(oid_cur, au, 0u).x;
      let iad = textureLoad(oid_cur, ad, 0u).x;
      if (dau + dad - 2.0 * d > OUTLINE_STEP * 2.0 || (iau != id && dau > d) || (iad != id && dad > d)) {
        edge = true;
      }
    }
  }
  if (!(edge || silhouette)) {
    return 0u;
  }
  // the line: hidden by the translucent cover from HIDING_ALPHA, the rest scaled by the strength
  if (cover >= HIDING_ALPHA) {
    return 0u;
  }
  let f = select(SILHOUETTE, 1.0, edge);
  return u32(round_away(min(g.outline * f, 1.0) * f32(31u - cover) / 31.0 * 15.0));
}

struct FOutline {
  @location(0) line: vec4u,
};

@fragment
fn fs_outline(in: VFull) -> FOutline {
  var o: FOutline;
  o.line = vec4u(outline_level(vec2u(u32(floor(in.clip.x)), u32(floor(in.clip.y)))), 0u, 0u, 0u);
  return o;
}

// the shading (этап 7.2): the software engine's `shade_pass` — three rays back towards the light
// (up and a little left), up to `shade_steps` pixels; one that meets something nearer by JUMP of
// depth shades the pixel, the nearer the stronger (a penumbra, the rays apart). Only what lies
// behind an object is shaded, never the object itself; interface, translucent-depth and empty
// pixels are not occluders, and a translucent cover weakens the shade like the outline's.
const JUMP: f32 = 0.006;

// one of the three rays, from the uniform (the CPU computed them exactly as the software engine)
fn shade_ray(r: u32) -> vec2f {
  if (r == 0u) {
    return vec2f(g.ray0x, g.ray0y);
  }
  if (r == 1u) {
    return vec2f(g.ray1x, g.ray1y);
  }
  return vec2f(g.ray2x, g.ray2y);
}

fn shade_level(p: vec2u) -> u32 {
  let alpha = (textureLoad(color, p, 0u).x >> 18u) & 31u;
  let d = textureLoad(depth_tex, p, 0u).x;
  let cover = textureLoad(cover_cur, p, 0u).x;
  if (alpha < 31u || textureLoad(trans_cur, p, 0u).x != 0u || d < INTERFACE_DEPTH || cover >= HIDING_ALPHA) {
    return 0u;
  }
  var total = 0.0;
  for (var r = 0u; r < 3u; r++) {
    let ray = shade_ray(r);
    for (var n = 1u; n <= g.shade_steps; n++) {
      let q = vec2i(i32(p.x) + i32(round_away(ray.x * f32(n))), i32(p.y) + i32(round_away(ray.y * f32(n))));
      if (q.x < 0 || q.y < 0 || q.x >= i32(g.w) || q.y >= i32(g.h)) {
        break;
      }
      let qq = vec2u(u32(q.x), u32(q.y));
      let qa = (textureLoad(color, qq, 0u).x >> 18u) & 31u;
      if (qa < 31u || textureLoad(trans_cur, qq, 0u).x != 0u || textureLoad(depth_tex, qq, 0u).x < INTERFACE_DEPTH) {
        continue;
      }
      if (textureLoad(depth_tex, qq, 0u).x < d - JUMP) {
        total = total + (1.0 - div(f32(n - 1u), f32(g.shade_steps)));
        break;
      }
    }
  }
  let dim = div(f32(31u - cover), 31.0);
  return u32(round_away(div(total, 3.0) * min(g.shadow, 1.0) * dim * 15.0));
}

struct FShade {
  @location(0) shade: vec4u,
};

@fragment
fn fs_shade(in: VFull) -> FShade {
  var o: FShade;
  o.shade = vec4u(shade_level(vec2u(u32(floor(in.clip.x)), u32(floor(in.clip.y)))), 0u, 0u, 0u);
  return o;
}

// the widened view's thresholds (этап 7.4), the software engine's: a row's source column is the
// first sky (depth >= SKY) in from the inner edge, else the first far (>= FAR)
const FAR: f32 = 0.999;
const SKY: f32 = 0.99996;

// the row passes' vertex shader: the quad's x goes 0..2 (one texel per side), the y 0..h
@vertex
fn vs_rows(in: VIn) -> VFull {
  var o: VFull;
  o.clip = vec4f(in.pos.x - 1.0, 1.0 - in.pos.y / f32(g.h) * 2.0, 0.0, 1.0);
  return o;
}

// the source column per row and side (этап 7.4, pass 1 of 3), two texels a row
@fragment
fn fs_rows(in: VFull) -> @location(0) vec4u {
  let side = u32(floor(in.clip.x));
  let y = u32(floor(in.clip.y));
  var sky_found = 0u;
  var sky_col = 0u;
  for (var k = 0u; k < g.inner; k++) {
    let x = select(g.e + k, g.w - g.e - 1u - k, side == 1u);
    if (textureLoad(depth_tex, vec2u(x, y), 0u).x >= SKY) {
      sky_found = 1u;
      sky_col = x;
      break;
    }
  }
  var far_found = 0u;
  var far_col = 0u;
  if (sky_found == 0u) {
    for (var k = 0u; k < g.inner; k++) {
      let x = select(g.e + k, g.w - g.e - 1u - k, side == 1u);
      if (textureLoad(depth_tex, vec2u(x, y), 0u).x >= FAR) {
        far_found = 1u;
        far_col = x;
        break;
      }
    }
  }
  return vec4u(sky_col | sky_found << 15u, far_col | far_found << 15u, 0u, 0u);
}

// the nearest row with a sky column (этап 7.4, pass 2 of 3): the row itself, then up and down
// alternately, above first — a row without its own sky fills with that row's sky column
@fragment
fn fs_skyrow(in: VFull) -> @location(0) vec4u {
  let side = u32(floor(in.clip.x));
  let y = u32(floor(in.clip.y));
  var r = 0u;
  var found = 0u;
  for (var d = 0u; d < g.h; d++) {
    if (y >= d) {
      let q = y - d;
      if (((textureLoad(srcinfo_tex, vec2u(side, q), 0u).r >> 15u) & 1u) != 0u) {
        r = q;
        found = 1u;
        break;
      }
    }
    if (y + d < g.h) {
      let q = y + d;
      if (((textureLoad(srcinfo_tex, vec2u(side, q), 0u).r >> 15u) & 1u) != 0u) {
        r = q;
        found = 1u;
        break;
      }
    }
  }
  let own = textureLoad(srcinfo_tex, vec2u(side, y), 0u);
  let sr_col = select(0u, textureLoad(srcinfo_tex, vec2u(side, r), 0u).r & 0x7FFFu, found != 0u);
  return vec4u(own.r, own.g, r | found << 15u, sr_col);
}

// the widened view's fill (этап 7.4, pass 3 of 3): a clear side pixel takes the row's source —
// its own sky column, else the nearest sky row's, else its own far column — the whole pixel,
// every field the passes after read; the rest of the picture carries over unchanged
struct FExtend {
  @location(0) word: vec4u,
  @location(1) fog: vec4u,
  @location(2) depth_out: vec4f,
  @location(3) id: vec4u,
  @location(4) oid: vec4u,
  @location(5) cover: vec4u,
  @location(6) trans: vec4u,
};

@fragment
fn fs_extend(in: VFull) -> FExtend {
  let p = vec2u(u32(floor(in.clip.x)), u32(floor(in.clip.y)));
  var src = p;
  if (g.extend_on != 0u && (p.x < g.e || p.x >= g.w - g.e) && textureLoad(depth_tex, p, 0u).x >= g.cd) {
    let side = select(0u, 1u, p.x >= g.w - g.e);
    let info = textureLoad(srcinfo_tex, vec2u(side, p.y), 0u);
    if (((info.r >> 15u) & 1u) != 0u) {
      src = vec2u(info.r & 0x7FFFu, p.y);
    } else if (((info.b >> 15u) & 1u) != 0u) {
      src = vec2u(info.a & 0x7FFFu, info.b & 0xFFFu);
    } else if (((info.g >> 15u) & 1u) != 0u) {
      src = vec2u(info.g & 0x7FFFu, p.y);
    }
  }
  var o: FExtend;
  o.word = vec4u(textureLoad(color, src, 0u).x, 0u, 0u, 0u);
  o.fog = vec4u(textureLoad(fogtex, src, 0u).x, 0u, 0u, 0u);
  o.depth_out = vec4f(textureLoad(depth_tex, src, 0u).x, 0.0, 0.0, 0.0);
  o.id = vec4u(textureLoad(id_cur, src, 0u).x, 0u, 0u, 0u);
  o.oid = vec4u(textureLoad(oid_cur, src, 0u).x, 0u, 0u, 0u);
  o.cover = vec4u(textureLoad(cover_cur, src, 0u).x, 0u, 0u, 0u);
  o.trans = vec4u(textureLoad(trans_cur, src, 0u).x, 0u, 0u, 0u);
  return o;
}

// a neighbour at `q` is of another polygon ID and not nearer: the pixel takes its edge colour
fn edge_neighbour(q: vec2u, id: u32, d: f32) -> bool {
  return textureLoad(id_cur, q, 0u).x != id && textureLoad(depth_tex, q, 0u).x >= d;
}

// the edge colour of a polygon ID: EDGE_COLOR[(id >> 3) & 7], two BGR555 words per scalar
fn edge_colour(id: u32) -> u32 {
  let ei = (id >> 3u) & 7u;
  var e: u32;
  if (ei < 2u) {
    e = g.edge0;
  } else if (ei < 4u) {
    e = g.edge1;
  } else if (ei < 6u) {
    e = g.edge2;
  } else {
    e = g.edge3;
  }
  let c = select(e & 0xFFFFu, e >> 16u, (ei & 1u) != 0u);
  return c5to6(c & 31u) | c5to6((c >> 5u) & 31u) << 6u | c5to6((c >> 10u) & 31u) << 12u;
}

@fragment
fn fs_fog(in: VFull) -> FFog {
  let p = vec2u(u32(floor(in.clip.x)), u32(floor(in.clip.y)));
  let word = textureLoad(color, p, 0u).x;
  var cr = word & 63u;
  var cg = (word >> 6u) & 63u;
  var cb = (word >> 12u) & 63u;
  var ca = (word >> 18u) & 31u;
  // the DS's edge marking (этап 7.3): an opaque pixel beside a farther-or-equal pixel of another
  // polygon ID takes its ID's edge colour — before the fog mixes the colour
  if (g.edge_on != 0u && ca == 31u) {
    let id = textureLoad(id_cur, p, 0u).x;
    let d = textureLoad(depth_tex, p, 0u).x;
    var marked = false;
    if (p.x >= g.scale) {
      marked = edge_neighbour(vec2u(p.x - g.scale, p.y), id, d);
    }
    if (!marked && p.x + g.scale < g.w) {
      marked = edge_neighbour(vec2u(p.x + g.scale, p.y), id, d);
    }
    if (!marked && p.y >= g.scale) {
      marked = edge_neighbour(vec2u(p.x, p.y - g.scale), id, d);
    }
    if (!marked && p.y + g.scale < g.h) {
      marked = edge_neighbour(vec2u(p.x, p.y + g.scale), id, d);
    }
    if (marked) {
      let ec = edge_colour(id);
      cr = ec & 63u;
      cg = (ec >> 6u) & 63u;
      cb = (ec >> 12u) & 63u;
    }
  }
  if (g.fog_enable != 0u && textureLoad(fogtex, p, 0u).x != 0u) {
    let depth = textureLoad(depth_tex, p, 0u).x;
    let depth24 = u32(clamp(depth, 0.0, 1.0) * 16777215.0);
    let d = fog_density(depth24);
    let fr = g.fog_color & 63u;
    let fg = (g.fog_color >> 6u) & 63u;
    let fb = (g.fog_color >> 12u) & 63u;
    let fa = (g.fog_color >> 18u) & 31u;
    if (g.fog_alpha_only == 0u) {
      cr = fmix(cr, fr, d);
      cg = fmix(cg, fg, d);
      cb = fmix(cb, fb, d);
    }
    ca = fmix(ca, fa, d);
  }
  var o: FFog;
  if (ca == 0u) {
    o.word = vec4u(0u, 0u, 0u, 0u);
  } else {
    let line = textureLoad(outline_tex, p, 0u).x & 15u;
    let shade = textureLoad(shade_tex, p, 0u).x & 15u;
    o.word = vec4u(cr | cg << 6u | cb << 12u | ca << 18u | line << 24u | shade << 28u, 0u, 0u, 0u);
  }
  return o;
}
"#;

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;
    use super::super::render3d::{GxPoly, Job};

    fn empty_job() -> Job {
        Job {
            verts: vec![],
            polys: vec![],
            wbuffer: false,
            disp3dcnt: 0,
            width: 256,
            clear_color: 31 << 16,
            clear_depth: 0x7FFF,
            clrimage_offset: 0,
            alpha_ref: 0,
            extras: super::super::Extras::default(),
            edge: [0; 8],
            fog_color: 31 << 16,
            fog_offset: 0,
            fog_table: [0; 32],
            toon: [0; 32],
            tex: vec![0; 0x80000],
            texpal: vec![0; 0x18000],
            keys: HashMap::new(),
            pack: Arc::new(super::super::texpack::Pack::empty()),
        }
    }

    fn vert(sx: f32, sy: f32, z: i32, w: i32, r: f32, g: f32, b: f32) -> GxVertex {
        GxVertex { x: 0, y: 0, z, w, sx, sy, ix: sx as i16, iy: sy as i16, r, g, b, s: 0.0, t: 0.0 }
    }

    /// A one-polygon job; `attr` carries alpha 31 (opaque) unless it says otherwise.
    fn job_with(verts: Vec<GxVertex>, n: u32, attr: u32) -> Job {
        let mut j = empty_job();
        j.verts = verts;
        j.polys = vec![GxPoly { first: 0, n, attr, tex: 0, pal: 0, translucent: false, sortkey: 0 }];
        j
    }

    /// A vertex over the screen with texture coordinates; colour white.
    fn texvert(sx: f32, sy: f32, s: f32, t: f32) -> GxVertex {
        GxVertex { x: 0, y: 0, z: 0, w: 0x1000, sx, sy, ix: sx as i16, iy: sy as i16, r: 31.0, g: 31.0, b: 31.0, s, t }
    }

    /// A texture's TEXIMAGE_PARAM word: format, size (8 << n), base slot, flags.
    fn tword(fmt: u32, size: u32, base: u32, flags: u32) -> u32 {
        fmt << 26 | size << 23 | size << 20 | base | flags
    }

    /// One full-screen textured polygon (white vertices; textures on).
    fn textured_job(tex: u32, pal: u32, attr: u32, st: [(f32, f32); 4]) -> Job {
        let mut j = empty_job();
        j.disp3dcnt |= 1;
        j.verts = vec![
            texvert(0.0, 0.0, st[0].0, st[0].1),
            texvert(255.0, 0.0, st[1].0, st[1].1),
            texvert(255.0, 191.0, st[2].0, st[2].1),
            texvert(0.0, 191.0, st[3].0, st[3].1),
        ];
        j.polys = vec![GxPoly { first: 0, n: 4, attr, tex, pal, translucent: false, sortkey: 0 }];
        j
    }

    /// Both engines draw the job; if a GPU device exists, the words must come out identical.
    fn both(job: &Job, scale: usize) -> Option<Vec<u32>> {
        let (want, wnotes) = render3d::render(job, scale);
        let gpu = match render(job, scale) {
            Ok((px, notes)) => {
                assert_eq!(notes, wnotes);
                px
            }
            Err(e) => {
                eprintln!("GPU test skipped: {e}");
                return None;
            }
        };
        let diffs: Vec<(usize, &u32, &u32)> = gpu
            .iter()
            .zip(&want)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .take(8)
            .map(|(i, (a, b))| (i, a, b))
            .collect();
        if !diffs.is_empty() {
            let mut msg = format!("the GPU picture differs from the software one at {} pixels, e.g.", gpu.iter().zip(&want).filter(|(a, b)| a != b).count());
            for (i, a, b) in diffs {
                msg.push_str(&format!(
                    "\n  pixel {} (x {}, y {}): {:#010x} vs {:#010x}",
                    i,
                    i % (job.width * scale),
                    i / (job.width * scale),
                    a,
                    b
                ));
            }
            panic!("{msg}");
        }
        Some(want)
    }

    /// Both engines draw the job and their words, notes and auxiliary maps must all match.
    fn both_maps(job: &Job, scale: usize) -> Option<()> {
        let (want, want_aux, wnotes) = super::super::render3d::render_maps(job, scale);
        let (got, oid, cover, trans, depth, notes) = match render_maps(job, scale) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("GPU test skipped: {e}");
                return None;
            }
        };
        assert_eq!(notes, wnotes, "notes differ");
        let diffs: Vec<(usize, &u32, &u32)> = got
            .iter()
            .zip(&want)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .take(8)
            .map(|(i, (a, b))| (i, a, b))
            .collect();
        if !diffs.is_empty() {
            let mut msg = format!("words differ at {} pixels, e.g.", got.iter().zip(&want).filter(|(a, b)| a != b).count());
            for (i, a, b) in diffs {
                msg.push_str(&format!("\n  pixel {i}: gpu {a:#010x} soft {b:#010x}"));
            }
            panic!("{msg}");
        }
        assert_eq!(oid, want_aux.oid, "oid map differs");
        assert_eq!(cover, want_aux.cover, "cover map differs");
        assert_eq!(trans, want_aux.trans, "trans map differs");
        assert_eq!(depth, want_aux.depth, "depth map differs");
        Some(())
    }

    #[test]
    fn a_triangle_with_fractional_edges_lands_on_the_exact_pixels() {
        let j = job_with(
            vec![
                vert(10.3, 20.7, 0, 0x1000, 31.0, 0.0, 0.0),
                vert(90.1, 30.4, 0, 0x1000, 0.0, 31.0, 0.0),
                vert(40.9, 160.8, 0, 0x1000, 0.0, 0.0, 31.0),
            ],
            3,
            31 << 16,
        );
        both(&j, 1);
        both(&j, 2);
    }

    #[test]
    fn two_triangles_sharing_a_diagonal_leave_no_gap_and_no_double_draw() {
        // a quad split along its diagonal: the top-left rule gives each edge pixel to one side
        let j = job_with(
            vec![
                vert(20.0, 20.0, 0, 0x1000, 31.0, 0.0, 0.0),
                vert(100.0, 20.0, 0, 0x1000, 31.0, 0.0, 0.0),
                vert(100.0, 100.0, 0, 0x1000, 31.0, 0.0, 0.0),
                vert(20.0, 100.0, 0, 0x1000, 31.0, 0.0, 0.0),
            ],
            4,
            31 << 16,
        );
        both(&j, 1);
    }

    #[test]
    fn the_nearer_polygon_wins_and_equal_depth_goes_to_the_first() {
        // a far square, then a near square over part of it (order as the engine drew it)
        let mut j = empty_job();
        j.verts = vec![
            vert(10.0, 10.0, 0x2000, 0x1000, 31.0, 0.0, 0.0),
            vert(150.0, 10.0, 0x2000, 0x1000, 31.0, 0.0, 0.0),
            vert(150.0, 150.0, 0x2000, 0x1000, 31.0, 0.0, 0.0),
            vert(10.0, 150.0, 0x2000, 0x1000, 31.0, 0.0, 0.0),
            vert(50.0, 50.0, 0x1000, 0x1000, 0.0, 31.0, 0.0),
            vert(120.0, 50.0, 0x1000, 0x1000, 0.0, 31.0, 0.0),
            vert(120.0, 120.0, 0x1000, 0x1000, 0.0, 31.0, 0.0),
            vert(50.0, 120.0, 0x1000, 0x1000, 0.0, 31.0, 0.0),
        ];
        j.polys = vec![
            GxPoly { first: 0, n: 4, attr: 31 << 16, tex: 0, pal: 0, translucent: false, sortkey: 0 },
            GxPoly { first: 4, n: 4, attr: 31 << 16, tex: 0, pal: 0, translucent: false, sortkey: 0 },
        ];
        both(&j, 1);
    }

    #[test]
    fn the_w_buffer_and_the_toon_table_match() {
        let mut j = job_with(
            vec![
                vert(10.0, 10.0, 0, 0x2000, 31.0, 0.0, 0.0),
                vert(200.0, 10.0, 0, 0x1000, 0.0, 31.0, 0.0),
                vert(100.0, 170.0, 0, 0x3000, 0.0, 0.0, 31.0),
            ],
            3,
            31 << 16 | 2 << 4,
        );
        j.wbuffer = true;
        // near the red vertex r6 = 63 (index 31); nearer the others the red falls away
        j.toon[31] = 0x7C00;
        j.toon[0] = 0x001F;
        both(&j, 1);
    }

    #[test]
    fn a_clockwise_fan_triangle_turns_and_draws_the_same_pixels() {
        // wound the wrong way: the engines swap two vertices (the negative area must not leak
        // into the interpolation)
        let j = job_with(
            vec![
                vert(20.0, 20.0, 0, 0x1000, 31.0, 0.0, 0.0),
                vert(20.0, 100.0, 0, 0x1000, 0.0, 31.0, 0.0),
                vert(100.0, 100.0, 0, 0x1000, 0.0, 0.0, 31.0),
            ],
            3,
            31 << 16,
        );
        both(&j, 1);
    }

    #[test]
    fn a_256_colour_texture_modulates_the_vertex_colour() {
        let st = [(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)];
        let mut j = textured_job(tword(4, 0, 0, 0), 0, 31 << 16, st);
        for i in 0..64 {
            j.tex[i] = i as u8;
        }
        for i in 0..64 {
            let c = (i & 31) | ((i * 3 & 31) << 5) | ((i * 5 & 31) << 10);
            j.texpal[2 * i..2 * i + 2].copy_from_slice(&(c as u16).to_le_bytes());
        }
        both(&j, 1);
        both(&j, 2);
    }

    #[test]
    fn a_transparent_texel_of_a_4_colour_texture_is_not_drawn() {
        let st = [(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)];
        let mut j = textured_job(tword(2, 0, 0, 0x2000_0000), 0, 31 << 16, st);
        for i in 0..16 {
            j.tex[i] = 0xE4; // texels 0,1,2,3: transparent, red, green, blue
        }
        for (i, c) in [0x001Fu16, 0x03E0u16, 0x7C00u16, 0x4210u16].into_iter().enumerate() {
            j.texpal[2 * i..2 * i + 2].copy_from_slice(&c.to_le_bytes());
        }
        both(&j, 1);
    }

    #[test]
    fn a_16_colour_texture_packs_two_texels_a_byte() {
        let st = [(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)];
        let mut j = textured_job(tword(3, 0, 0, 0), 0, 31 << 16, st);
        for i in 0..32 {
            j.tex[i] = 0x10; // the texels alternate between palette colours 0 and 1
        }
        for (i, c) in [0x001Fu16, 0x03E0u16].into_iter().enumerate() {
            j.texpal[2 * i..2 * i + 2].copy_from_slice(&c.to_le_bytes());
        }
        both(&j, 1);
    }

    #[test]
    fn a_compressed_texture_draws_each_tile_by_its_mode() {
        let st = [(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)];
        let mut j = textured_job(tword(5, 0, 0, 0), 0, 31 << 16, st);
        // 8x8 is four 4x4 tiles, each reading texels 0,1,2,3; the tiles' words pick their palette
        // blocks (in 4-byte units, 8 bytes apart) and their interpolation mode
        for t in 0..4 {
            j.tex[t * 4..t * 4 + 4].copy_from_slice(&[0xE4, 0xE4, 0xE4, 0xE4]);
        }
        for (t, word) in [0x0000u16, 0x4002, 0x8004, 0xC006].into_iter().enumerate() {
            j.tex[0x20000 + t * 2..0x20000 + t * 2 + 2].copy_from_slice(&word.to_le_bytes());
        }
        for t in 0..4 {
            for c in 0..4 {
                let v = 4 + t * 4 + c;
                let col = (v & 31) | ((v * 3 & 31) << 5) | ((v * 5 & 31) << 10);
                j.texpal[t * 8 + c * 2..t * 8 + c * 2 + 2].copy_from_slice(&(col as u16).to_le_bytes());
            }
        }
        both(&j, 1);
    }

    #[test]
    fn a_direct_colour_texture_takes_its_alpha_from_bit_15() {
        let st = [(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)];
        let mut j = textured_job(tword(7, 0, 0, 0), 0, 31 << 16, st);
        for i in 0..64 {
            let c: u16 = if i & 1 == 0 { 0x001F } else { 0x001F | 0x8000 }; // red, half of it transparent
            j.tex[2 * i..2 * i + 2].copy_from_slice(&c.to_le_bytes());
        }
        both(&j, 1);
    }

    #[test]
    fn texture_coordinates_wrap_repeat_and_flip_like_the_software() {
        // repeat and flip on both axes, the coordinates far outside the texture (negative and
        // over the size, so the mirror parity of negative runs is exercised)
        let st = [(-16.0, -8.0), (40.0, -8.0), (40.0, 24.0), (-16.0, 24.0)];
        let mut j = textured_job(tword(4, 0, 0, 0x10000 | 0x20000 | 0x40000 | 0x80000), 0, 31 << 16, st);
        for i in 0..64 {
            j.tex[i] = i as u8;
        }
        for i in 0..64 {
            let c = (i & 31) | ((i * 3 & 31) << 5) | ((i * 5 & 31) << 10);
            j.texpal[2 * i..2 * i + 2].copy_from_slice(&(c as u16).to_le_bytes());
        }
        both(&j, 1);
    }

    #[test]
    fn the_decal_mode_puts_the_texture_over_the_vertex_colour() {
        // the left half of the texture is transparent (the vertex colour shows), the right half
        // opaque red; the vertices are green
        let st = [(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)];
        let mut j = textured_job(tword(7, 0, 0, 0), 0, 31 << 16 | 1 << 4, st);
        for i in 0..64 {
            let c: u16 = if i & 7 < 4 { 0x03E0 } else { 0x001F | 0x8000 };
            j.tex[2 * i..2 * i + 2].copy_from_slice(&c.to_le_bytes());
        }
        for v in &mut j.verts {
            v.r = 0.0;
            v.g = 31.0;
            v.b = 0.0;
        }
        both(&j, 1);
    }

    #[test]
    fn the_toon_highlight_adds_the_table_colour_over_the_texture() {
        // a black texture: the highlight's gray modulates to black, then the table's red is added
        let st = [(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)];
        let mut j = textured_job(tword(4, 0, 0, 0), 0, 31 << 16 | 2 << 4, st);
        j.disp3dcnt |= 2;
        for i in 0..64 {
            j.tex[i] = 0;
        }
        j.texpal[0..2].copy_from_slice(&0u16.to_le_bytes()); // black
        j.toon[31] = 0x001F; // red: the vertex colour's red (63) picks index 31
        both(&j, 1);
    }

    #[test]
    fn a_pack_picture_replaces_the_texture_over_the_same_coordinates() {
        let st = [(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)];
        let mut j = textured_job(tword(4, 0, 0, 0), 0, 31 << 16, st);
        let key = TexKey { data: 0x1111, pal: 0x2222, fmt: 4, w: 8, h: 8 };
        j.keys.insert((j.polys[0].tex, j.polys[0].pal), key);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tiny.spdata");
        j.pack = Arc::new(super::super::texpack::Pack::from_file(&path).unwrap());
        both(&j, 1);
    }

    #[test]
    fn the_bilinear_filter_blends_by_alpha_and_matches_the_software() {
        let st = [(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)];
        // a lone red texel in a sea of transparent ones: the alpha-weighted blend keeps the red
        // where the taps see it, and nowhere else
        let mut j = textured_job(tword(4, 0, 0, 0x2000_0000), 0, 31 << 16, st);
        j.extras.filter = super::super::TexFilter::Bilinear;
        j.tex[0] = 1;
        j.texpal[2..4].copy_from_slice(&0x001Fu16.to_le_bytes()); // red at index 1
        both(&j, 1);
        // a gradient: the same blend over all texels, the whole picture at once
        let mut j = textured_job(tword(4, 0, 0, 0), 0, 31 << 16, st);
        j.extras.filter = super::super::TexFilter::Bilinear;
        for i in 0..64 {
            j.tex[i] = i as u8;
        }
        for i in 0..64 {
            let c = (i & 31) | ((i * 3 & 31) << 5) | ((i * 5 & 31) << 10);
            j.texpal[2 * i..2 * i + 2].copy_from_slice(&(c as u16).to_le_bytes());
        }
        both(&j, 1);
    }

    #[test]
    fn the_alpha_test_cuts_the_texture_by_its_alpha() {
        let st = [(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)];
        // ref 0: only the transparent texels go (they would anyway); ref 31: the whole polygon goes
        for aref in [0u8, 31] {
            let mut j = textured_job(tword(7, 0, 0, 0), 0, 31 << 16, st);
            j.disp3dcnt |= 4;
            j.alpha_ref = aref;
            for i in 0..64 {
                let c: u16 = if i & 1 == 0 { 0x001F } else { 0x001F | 0x8000 };
                j.tex[2 * i..2 * i + 2].copy_from_slice(&c.to_le_bytes());
            }
            both(&j, 1);
        }
    }

    #[test]
    fn textured_and_plain_polygons_share_a_frame() {
        // a plain red square on the left, a textured square on the right
        let mut j = empty_job();
        j.disp3dcnt |= 1;
        j.verts = vec![
            vert(10.0, 10.0, 0, 0x1000, 31.0, 0.0, 0.0),
            vert(100.0, 10.0, 0, 0x1000, 31.0, 0.0, 0.0),
            vert(100.0, 100.0, 0, 0x1000, 31.0, 0.0, 0.0),
            vert(10.0, 100.0, 0, 0x1000, 31.0, 0.0, 0.0),
            texvert(150.0, 10.0, 0.0, 0.0),
            texvert(240.0, 10.0, 8.0, 0.0),
            texvert(240.0, 100.0, 8.0, 8.0),
            texvert(150.0, 100.0, 0.0, 8.0),
        ];
        j.polys = vec![
            GxPoly { first: 0, n: 4, attr: 31 << 16, tex: 0, pal: 0, translucent: false, sortkey: 0 },
            GxPoly { first: 4, n: 4, attr: 31 << 16, tex: tword(4, 0, 0, 0), pal: 0, translucent: false, sortkey: 0 },
        ];
        // a green 256-colour texture, index 0
        j.texpal[0..2].copy_from_slice(&0x03E0u16.to_le_bytes());
        both(&j, 1);
    }

    #[test]
    fn the_clear_plane_alone_is_the_clear_word() {
        let j = empty_job();
        both(&j, 1);
    }

    #[test]
    fn fog_moves_the_clear_plane_to_the_fog_colour_and_back() {
        const BLUE: u32 = 0x7C00;
        const RED: u32 = 0x001F;
        // full density: the clear plane (taking the fog) becomes the fog colour
        let mut j = empty_job();
        j.disp3dcnt = 0x80;
        j.clear_color = BLUE | 0x8000 | 31 << 16;
        j.fog_color = RED | 31 << 16;
        j.fog_table = [127; 32];
        both(&j, 1);
        // half density: half way to the fog colour
        j.fog_table = [64; 32];
        both(&j, 1);
        // alpha only to a fog alpha of 0: the colour stays, the pixel goes transparent
        j.disp3dcnt = 0xC0;
        j.fog_color = RED;
        j.fog_table = [127; 32];
        both(&j, 1);
        // without the plane's fog bit nothing changes
        j.disp3dcnt = 0x80;
        j.clear_color = BLUE | 31 << 16;
        j.fog_color = RED | 31 << 16;
        both(&j, 1);
    }

    #[test]
    fn a_polygon_takes_the_fog_by_its_own_bit_and_depth() {
        const RED: u32 = 0x001F;
        // a triangle whose depth varies across it (so the density steps the table), fog on
        let mut j = job_with(
            vec![
                vert(10.0, 10.0, 0x0000, 0x1000, 31.0, 0.0, 0.0),
                vert(200.0, 10.0, 0x1000, 0x1000, 0.0, 31.0, 0.0),
                vert(100.0, 170.0, 0x0800, 0x1000, 0.0, 0.0, 31.0),
            ],
            3,
            31 << 16 | 0x8000,
        );
        j.disp3dcnt = 0x80;
        j.fog_color = RED | 31 << 16;
        for i in 0..32 {
            j.fog_table[i] = (i * 4) as u8;
        }
        both(&j, 1);
        // the same frame with the fog shifted and the steps halved
        j.fog_offset = 0x100;
        j.disp3dcnt = 0x80 | 1 << 8;
        both(&j, 1);
    }

    #[test]
    fn the_rear_plane_bitmap_replaces_the_clear_colour_and_scrolls() {
        const RED: u16 = 0x001F;
        let mut j = empty_job();
        j.disp3dcnt = 0x4000;
        j.clear_color = 31 << 16;
        // slot 2: pixel (1, 0) opaque red; slot 3: its depth 0 (nearest)
        let put = |tex: &mut Vec<u8>, base: usize, x: usize, v: u16| {
            tex[base + 2 * x..base + 2 * x + 2].copy_from_slice(&v.to_le_bytes());
        };
        put(&mut j.tex, 0x40000, 1, RED | 0x8000);
        put(&mut j.tex, 0x60000, 1, 0);
        both(&j, 1);
        // scrolled by one, the red pixel is at x = 0
        j.clrimage_offset = 1;
        both(&j, 1);
        // drawn at scale 2 the pixel is two wide
        j.clrimage_offset = 0;
        both(&j, 2);
    }

    #[test]
    fn a_polygon_draws_over_the_rear_plane_by_its_depth() {
        const GREEN: u16 = 0x03E0;
        let mut j = empty_job();
        j.disp3dcnt = 0x4000;
        j.clear_color = 31 << 16;
        // the whole bitmap: an opaque green pixel (the far depth) everywhere via slot 2
        for x in 0..(256 * 256) {
            j.tex[0x40000 + 2 * x..0x40000 + 2 * x + 2].copy_from_slice(&(GREEN | 0x8000).to_le_bytes());
            j.tex[0x60000 + 2 * x..0x60000 + 2 * x + 2].copy_from_slice(&0x7FFFu16.to_le_bytes()); // far depth
        }
        // a nearer red square drawn over part of it
        j.verts = vec![
            vert(10.0, 10.0, 0, 0x1000, 31.0, 0.0, 0.0),
            vert(100.0, 10.0, 0, 0x1000, 31.0, 0.0, 0.0),
            vert(100.0, 100.0, 0, 0x1000, 31.0, 0.0, 0.0),
            vert(10.0, 100.0, 0, 0x1000, 31.0, 0.0, 0.0),
        ];
        j.polys = vec![GxPoly { first: 0, n: 4, attr: 31 << 16, tex: 0, pal: 0, translucent: false, sortkey: 0 }];
        both(&j, 1);
    }

    #[test]
    fn a_translucent_polygon_blends_over_what_is_there() {
        // an opaque red square, then a half-alpha blue square over part of it
        let mut j = empty_job();
        j.disp3dcnt |= 8; // blending on
        j.verts = vec![
            vert(10.0, 10.0, 0, 0x1000, 31.0, 0.0, 0.0),
            vert(150.0, 10.0, 0, 0x1000, 31.0, 0.0, 0.0),
            vert(150.0, 150.0, 0, 0x1000, 31.0, 0.0, 0.0),
            vert(10.0, 150.0, 0, 0x1000, 31.0, 0.0, 0.0),
            vert(50.0, 50.0, 0, 0x1000, 0.0, 0.0, 31.0),
            vert(120.0, 50.0, 0, 0x1000, 0.0, 0.0, 31.0),
            vert(120.0, 120.0, 0, 0x1000, 0.0, 0.0, 31.0),
            vert(50.0, 120.0, 0, 0x1000, 0.0, 0.0, 31.0),
        ];
        j.polys = vec![
            GxPoly { first: 0, n: 4, attr: 31 << 16, tex: 0, pal: 0, translucent: false, sortkey: 0 },
            GxPoly { first: 4, n: 4, attr: 15 << 16, tex: 0, pal: 0, translucent: true, sortkey: 0 },
        ];
        both(&j, 1);
        both(&j, 2);
    }

    #[test]
    fn a_translucent_polygon_does_not_redraw_its_own_id() {
        // over a transparent clear, two overlapping translucent polygons of the same ID: the second
        // does not draw over the first (the software engine's rule)
        let mut j = empty_job();
        j.disp3dcnt |= 8;
        j.clear_color = 0; // transparent clear
        j.verts = vec![
            vert(40.0, 40.0, 0, 0x1000, 0.0, 31.0, 0.0),
            vert(160.0, 40.0, 0, 0x1000, 0.0, 31.0, 0.0),
            vert(160.0, 140.0, 0, 0x1000, 0.0, 31.0, 0.0),
            vert(40.0, 140.0, 0, 0x1000, 0.0, 31.0, 0.0),
            vert(80.0, 80.0, 0, 0x1000, 0.0, 31.0, 0.0),
            vert(180.0, 80.0, 0, 0x1000, 0.0, 31.0, 0.0),
            vert(180.0, 160.0, 0, 0x1000, 0.0, 31.0, 0.0),
            vert(80.0, 160.0, 0, 0x1000, 0.0, 31.0, 0.0),
        ];
        j.polys = vec![
            GxPoly { first: 0, n: 4, attr: 20 << 16 | 3 << 24, tex: 0, pal: 0, translucent: true, sortkey: 0 },
            GxPoly { first: 4, n: 4, attr: 20 << 16 | 3 << 24, tex: 0, pal: 0, translucent: true, sortkey: 0 },
        ];
        both(&j, 1);
    }

    #[test]
    fn a_shadow_mask_marks_and_the_shadow_draws() {
        // an opaque wall (id 5); a mask (mode 3, id 0) behind it marks the wall; a shadow (mode 3,
        // id 1) in front of it darkens it only where the mask marked
        let mut j = empty_job();
        j.disp3dcnt |= 8;
        j.verts = vec![
            vert(20.0, 20.0, 0, 0x1000, 31.0, 31.0, 31.0),
            vert(180.0, 20.0, 0, 0x1000, 31.0, 31.0, 31.0),
            vert(180.0, 150.0, 0, 0x1000, 31.0, 31.0, 31.0),
            vert(20.0, 150.0, 0, 0x1000, 31.0, 31.0, 31.0),
            vert(20.0, 20.0, 0x1000, 0x1000, 0.0, 0.0, 0.0),
            vert(180.0, 20.0, 0x1000, 0x1000, 0.0, 0.0, 0.0),
            vert(180.0, 150.0, 0x1000, 0x1000, 0.0, 0.0, 0.0),
            vert(20.0, 150.0, 0x1000, 0x1000, 0.0, 0.0, 0.0),
            vert(20.0, 20.0, -0x1000, 0x1000, 0.0, 0.0, 0.0),
            vert(180.0, 20.0, -0x1000, 0x1000, 0.0, 0.0, 0.0),
            vert(180.0, 150.0, -0x1000, 0x1000, 0.0, 0.0, 0.0),
            vert(20.0, 150.0, -0x1000, 0x1000, 0.0, 0.0, 0.0),
        ];
        j.polys = vec![
            GxPoly { first: 0, n: 4, attr: 31 << 16 | 5 << 24, tex: 0, pal: 0, translucent: false, sortkey: 0 },
            GxPoly { first: 4, n: 4, attr: 10 << 16 | 3 << 4, tex: 0, pal: 0, translucent: true, sortkey: 0 },
            GxPoly { first: 8, n: 4, attr: 10 << 16 | 3 << 4 | 1 << 24, tex: 0, pal: 0, translucent: true, sortkey: 0 },
        ];
        both(&j, 1);
    }

    #[test]
    fn a3i5_and_a5i3_textures_draw_translucent() {
        let st = [(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)];
        // A3I5: alpha 7 (full), colour index 0; the polygon's own alpha 15 modulates
        let mut j = textured_job(tword(1, 0, 0, 0), 0, 15 << 16, st);
        j.disp3dcnt |= 8;
        j.polys[0].translucent = true;
        for i in 0..64 {
            j.tex[i] = 0xE0;
        }
        j.texpal[0..2].copy_from_slice(&0x001Fu16.to_le_bytes());
        both(&j, 1);
        // A5I3: alpha 16, colour index 0
        let mut j = textured_job(tword(6, 0, 0, 0), 0, 15 << 16, st);
        j.disp3dcnt |= 8;
        j.polys[0].translucent = true;
        for i in 0..64 {
            j.tex[i] = 0x80;
        }
        j.texpal[0..2].copy_from_slice(&0x001Fu16.to_le_bytes());
        both(&j, 1);
    }

    #[test]
    fn a_translucent_polygon_writes_the_depth_with_bit_11() {
        // a translucent green square over the fog-taking clear, with the fog bit and the depth write
        // (bit 11): the fog uses the polygon's nearer depth, not the clear's far one
        const RED: u32 = 0x001F;
        let mut j = empty_job();
        j.disp3dcnt = 0x80 | 8; // fog on, blending on
        j.clear_color = 0x7C00 | 0x8000 | 31 << 16; // blue, fog bit, opaque
        j.fog_color = RED | 31 << 16;
        for i in 0..16 {
            j.fog_table[i] = 127;
        }
        j.verts = vec![
            vert(10.0, 10.0, 0, 0x1000, 0.0, 31.0, 0.0),
            vert(150.0, 10.0, 0, 0x1000, 0.0, 31.0, 0.0),
            vert(150.0, 150.0, 0, 0x1000, 0.0, 31.0, 0.0),
            vert(10.0, 150.0, 0, 0x1000, 0.0, 31.0, 0.0),
        ];
        j.polys = vec![GxPoly { first: 0, n: 4, attr: 15 << 16 | 0x800 | 0x8000, tex: 0, pal: 0, translucent: true, sortkey: 0 }];
        both(&j, 1);
    }

    #[test]
    fn the_auxiliary_maps_follow_the_opaque_and_translucent_polygons() {
        // an opaque square (id 5), a translucent square (id 7, alpha 15) over its right half, and a
        // translucent square (id 9, alpha 20, depth write) over part of both: `oid` stays the opaque
        // polygon's 5, `cover` takes the translucents' alphas, `trans` turns on where the depth is
        // written
        let mut j = empty_job();
        j.disp3dcnt |= 8;
        j.verts = vec![
            vert(10.0, 10.0, 0, 0x1000, 31.0, 0.0, 0.0),
            vert(150.0, 10.0, 0, 0x1000, 31.0, 0.0, 0.0),
            vert(150.0, 150.0, 0, 0x1000, 31.0, 0.0, 0.0),
            vert(10.0, 150.0, 0, 0x1000, 31.0, 0.0, 0.0),
            vert(80.0, 20.0, 0, 0x1000, 0.0, 0.0, 31.0),
            vert(170.0, 20.0, 0, 0x1000, 0.0, 0.0, 31.0),
            vert(170.0, 170.0, 0, 0x1000, 0.0, 0.0, 31.0),
            vert(80.0, 170.0, 0, 0x1000, 0.0, 0.0, 31.0),
            vert(90.0, 40.0, 0, 0x1000, 0.0, 31.0, 0.0),
            vert(180.0, 40.0, 0, 0x1000, 0.0, 31.0, 0.0),
            vert(180.0, 180.0, 0, 0x1000, 0.0, 31.0, 0.0),
            vert(90.0, 180.0, 0, 0x1000, 0.0, 31.0, 0.0),
        ];
        j.polys = vec![
            GxPoly { first: 0, n: 4, attr: 31 << 16 | 5 << 24, tex: 0, pal: 0, translucent: false, sortkey: 0 },
            GxPoly { first: 4, n: 4, attr: 15 << 16 | 7 << 24, tex: 0, pal: 0, translucent: true, sortkey: 0 },
            GxPoly { first: 8, n: 4, attr: 20 << 16 | 9 << 24 | 0x800, tex: 0, pal: 0, translucent: true, sortkey: 0 },
        ];
        both_maps(&j, 1);
        both_maps(&j, 2);
    }

    #[test]
    fn a_vertex_at_the_near_plane_and_a_degenerate_w_match_the_software() {
        // what GX's clipping leaves at the camera's boundary: a vertex exactly on the near plane
        // (z == -w: the depth a hair negative), a vertex with w <= 0 (both engines clamp it to 1),
        // and a far one; the far square under the triangle stays where the triangle is not nearer
        let mut j = empty_job();
        j.verts = vec![
            vert(20.0, 20.0, 0x0800, 0x1000, 31.0, 0.0, 0.0),
            vert(150.0, 20.0, 0x0800, 0x1000, 31.0, 0.0, 0.0),
            vert(150.0, 150.0, 0x0800, 0x1000, 31.0, 0.0, 0.0),
            vert(20.0, 150.0, 0x0800, 0x1000, 31.0, 0.0, 0.0),
            vert(40.0, 60.0, -0x1000, 0x1000, 0.0, 31.0, 0.0),
            vert(160.0, 60.0, 0, 0, 0.0, 31.0, 0.0),
            vert(100.0, 160.0, 0x1000, 0x1000, 0.0, 31.0, 0.0),
        ];
        j.polys = vec![
            GxPoly { first: 0, n: 4, attr: 31 << 16 | 3 << 24, tex: 0, pal: 0, translucent: false, sortkey: 0 },
            GxPoly { first: 4, n: 3, attr: 31 << 16 | 5 << 24, tex: 0, pal: 0, translucent: false, sortkey: 0 },
        ];
        both(&j, 1);
        both(&j, 2);
        both_maps(&j, 1);
    }

    #[test]
    fn equal_depth_keeps_the_first_polygon_in_the_job_order() {
        // two overlapping squares at exactly the same depth: the second is never nearer, so the
        // first stays everywhere they overlap (the job's order is the only tie-breaker)
        let mut j = empty_job();
        j.verts = vec![
            vert(20.0, 20.0, 0x1000, 0x1000, 31.0, 0.0, 0.0),
            vert(150.0, 20.0, 0x1000, 0x1000, 31.0, 0.0, 0.0),
            vert(150.0, 150.0, 0x1000, 0x1000, 31.0, 0.0, 0.0),
            vert(20.0, 150.0, 0x1000, 0x1000, 31.0, 0.0, 0.0),
            vert(100.0, 20.0, 0x1000, 0x1000, 0.0, 0.0, 31.0),
            vert(200.0, 20.0, 0x1000, 0x1000, 0.0, 0.0, 31.0),
            vert(200.0, 150.0, 0x1000, 0x1000, 0.0, 0.0, 31.0),
            vert(100.0, 150.0, 0x1000, 0x1000, 0.0, 0.0, 31.0),
        ];
        j.polys = vec![
            GxPoly { first: 0, n: 4, attr: 31 << 16 | 3 << 24, tex: 0, pal: 0, translucent: false, sortkey: 0 },
            GxPoly { first: 4, n: 4, attr: 31 << 16 | 5 << 24, tex: 0, pal: 0, translucent: false, sortkey: 0 },
        ];
        both(&j, 1);
        both(&j, 2);
        both_maps(&j, 1);
    }

    #[test]
    fn translucent_polygons_at_equal_depth_follow_the_job_order_unless_eq_depth() {
        // two translucent squares at the same depth, the second over the first (the first writes
        // its depth, bit 11): without the equal-depth test (bit 14) the second is behind and the
        // first stays; with it, the second draws over the first
        for eq in [false, true] {
            let mut j = empty_job();
            j.disp3dcnt |= 8;
            j.verts = vec![
                vert(40.0, 40.0, -0x0800, 0x1000, 0.0, 31.0, 0.0),
                vert(160.0, 40.0, -0x0800, 0x1000, 0.0, 31.0, 0.0),
                vert(160.0, 140.0, -0x0800, 0x1000, 0.0, 31.0, 0.0),
                vert(40.0, 140.0, -0x0800, 0x1000, 0.0, 31.0, 0.0),
                vert(80.0, 80.0, -0x0800, 0x1000, 31.0, 0.0, 0.0),
                vert(180.0, 80.0, -0x0800, 0x1000, 31.0, 0.0, 0.0),
                vert(180.0, 160.0, -0x0800, 0x1000, 31.0, 0.0, 0.0),
                vert(80.0, 160.0, -0x0800, 0x1000, 31.0, 0.0, 0.0),
            ];
            let eq_bit = if eq { 0x4000 } else { 0 };
            j.polys = vec![
                GxPoly { first: 0, n: 4, attr: 15 << 16 | 7 << 24 | 0x800, tex: 0, pal: 0, translucent: true, sortkey: 0 },
                GxPoly { first: 4, n: 4, attr: 15 << 16 | 9 << 24 | eq_bit, tex: 0, pal: 0, translucent: true, sortkey: 0 },
            ];
            both(&j, 1);
            both_maps(&j, 1);
        }
    }

    #[test]
    fn the_outline_matches_the_software_on_jumps_silhouettes_and_cover() {
        // a transparent clear (the 3D's border is a silhouette), a far square (id 3), a nearer
        // square (id 5: a jump in depth and a change of ID → a line on its near side), and a
        // translucent panel (id 9, alpha 15) over part of it: the cover from HIDING_ALPHA hides
        // the line under the panel
        let mut j = empty_job();
        j.clear_color = 0;
        j.disp3dcnt |= 8;
        j.extras.outline = 1.0;
        j.extras.outline_width = 0.5;
        j.verts = vec![
            vert(20.0, 20.0, 0x0800, 0x1000, 31.0, 0.0, 0.0),
            vert(150.0, 20.0, 0x0800, 0x1000, 31.0, 0.0, 0.0),
            vert(150.0, 150.0, 0x0800, 0x1000, 31.0, 0.0, 0.0),
            vert(20.0, 150.0, 0x0800, 0x1000, 31.0, 0.0, 0.0),
            vert(60.0, 40.0, -0x0800, 0x1000, 0.0, 31.0, 0.0),
            vert(120.0, 40.0, -0x0800, 0x1000, 0.0, 31.0, 0.0),
            vert(120.0, 120.0, -0x0800, 0x1000, 0.0, 31.0, 0.0),
            vert(60.0, 120.0, -0x0800, 0x1000, 0.0, 31.0, 0.0),
            vert(70.0, 50.0, -0x0C00, 0x1000, 31.0, 0.0, 31.0),
            vert(110.0, 50.0, -0x0C00, 0x1000, 31.0, 0.0, 31.0),
            vert(110.0, 110.0, -0x0C00, 0x1000, 31.0, 0.0, 31.0),
            vert(70.0, 110.0, -0x0C00, 0x1000, 31.0, 0.0, 31.0),
        ];
        j.polys = vec![
            GxPoly { first: 0, n: 4, attr: 31 << 16 | 3 << 24, tex: 0, pal: 0, translucent: false, sortkey: 0 },
            GxPoly { first: 4, n: 4, attr: 31 << 16 | 5 << 24, tex: 0, pal: 0, translucent: false, sortkey: 0 },
            GxPoly { first: 8, n: 4, attr: 15 << 16 | 9 << 24, tex: 0, pal: 0, translucent: true, sortkey: 0 },
        ];
        // the reference draws lines: the near square's border against the farther square and the
        // squares' borders against the transparent clear — but not under the hiding panel
        let (want, _, _) = super::super::render3d::render_maps(&j, 1);
        let lines = want.iter().filter(|w| *w >> 24 & 15 != 0).count();
        assert!(lines > 0, "the reference frame has no outline at all");
        both_maps(&j, 1);
        both_maps(&j, 2);
    }

    #[test]
    fn the_shading_matches_the_software_behind_an_occluder() {
        // a far square, a nearer square over its middle: from the far square's pixels just below
        // and right of the nearer one, a ray towards the light (up and a little left) meets the
        // nearer square and shades them — the shade level matches the software engine's
        let mut j = empty_job();
        j.extras.shadow = 1.0;
        j.extras.shadow_radius = 2.0;
        j.verts = vec![
            vert(20.0, 20.0, 0x0800, 0x1000, 31.0, 0.0, 0.0),
            vert(180.0, 20.0, 0x0800, 0x1000, 31.0, 0.0, 0.0),
            vert(180.0, 180.0, 0x0800, 0x1000, 31.0, 0.0, 0.0),
            vert(20.0, 180.0, 0x0800, 0x1000, 31.0, 0.0, 0.0),
            vert(60.0, 40.0, -0x0800, 0x1000, 0.0, 31.0, 0.0),
            vert(120.0, 40.0, -0x0800, 0x1000, 0.0, 31.0, 0.0),
            vert(120.0, 100.0, -0x0800, 0x1000, 0.0, 31.0, 0.0),
            vert(60.0, 100.0, -0x0800, 0x1000, 0.0, 31.0, 0.0),
        ];
        j.polys = vec![
            GxPoly { first: 0, n: 4, attr: 31 << 16 | 3 << 24, tex: 0, pal: 0, translucent: false, sortkey: 0 },
            GxPoly { first: 4, n: 4, attr: 31 << 16 | 5 << 24, tex: 0, pal: 0, translucent: false, sortkey: 0 },
        ];
        let (want, _, _) = super::super::render3d::render_maps(&j, 1);
        let shaded = want.iter().filter(|w| *w >> 28 & 15 != 0).count();
        assert!(shaded > 0, "the reference frame has no shading at all");
        both_maps(&j, 1);
        both_maps(&j, 2);
    }

    #[test]
    fn the_edge_marking_colours_a_polygon_border_by_its_id() {
        // a red square (id 3): its border against the clear plane takes the edge colour for
        // (3 >> 3) & 7 = 0 — green here — while the inside keeps the polygon's red
        let mut j = empty_job();
        j.disp3dcnt |= 0x20;
        j.edge = [0x03E0, 0x001F, 0, 0, 0, 0, 0, 0];
        j.verts = vec![
            vert(20.0, 20.0, 0x0800, 0x1000, 31.0, 0.0, 0.0),
            vert(150.0, 20.0, 0x0800, 0x1000, 31.0, 0.0, 0.0),
            vert(150.0, 150.0, 0x0800, 0x1000, 31.0, 0.0, 0.0),
            vert(20.0, 150.0, 0x0800, 0x1000, 31.0, 0.0, 0.0),
        ];
        j.polys = vec![GxPoly { first: 0, n: 4, attr: 31 << 16 | 3 << 24, tex: 0, pal: 0, translucent: false, sortkey: 0 }];
        let (want, _, _) = super::super::render3d::render_maps(&j, 1);
        // the edge colour (green: 63 in bits 6-11) appears along the border, the red stays inside
        assert!(want.iter().any(|w| *w & 0x3FFFF == 63 << 6), "the reference has no edge-marked pixels");
        assert!(want.iter().any(|w| *w & 0x3FFFF == 63), "the reference lost the polygon's own colour");
        both_maps(&j, 1);
        both_maps(&j, 2);
    }

    #[test]
    fn the_widened_view_fills_the_sides_from_the_far_plane() {
        // a wide frame (342 wide, the 4:3 is the middle 256): a far plane (depth 0.9995, between
        // FAR and SKY) covers the central columns and every row; the 43 columns at each side are
        // clear. The fill runs the far plane on: each clear side pixel takes the row's first far
        // column (43 on the left, 298 on the right).
        let mut j = empty_job();
        j.width = 342;
        j.verts = vec![
            vert(43.0, 0.0, 0x7FF0, 0x8000, 31.0, 0.0, 0.0),
            vert(299.0, 0.0, 0x7FF0, 0x8000, 31.0, 0.0, 0.0),
            vert(299.0, 192.0, 0x7FF0, 0x8000, 31.0, 0.0, 0.0),
            vert(43.0, 192.0, 0x7FF0, 0x8000, 31.0, 0.0, 0.0),
        ];
        j.polys = vec![GxPoly { first: 0, n: 4, attr: 31 << 16 | 3 << 24, tex: 0, pal: 0, translucent: false, sortkey: 0 }];
        let (want, _, _) = super::super::render3d::render_maps(&j, 1);
        for y in [0, 50, 191] {
            assert_eq!(want[y * 342 + 10], want[y * 342 + 43], "the left side was not filled from the far column");
            assert_eq!(want[y * 342 + 341], want[y * 342 + 298], "the right side was not filled from the far column");
        }
        // the side columns hold the far plane's red: the fill really happened
        assert_eq!(want[50 * 342 + 10] & 0x3FFFF, 63);
        both_maps(&j, 1);
        both_maps(&j, 2);
    }

    #[test]
    fn the_widened_view_fills_the_sides_from_the_nearest_sky_row() {
        // a wide frame: a sky plane (depth 0.99997, at least SKY) covers the central columns in
        // rows 0-79, a far ground (0.9995) below. The rows with the sky fill from their own sky
        // column; the rows below fill from the nearest row above with one (row 79), never from the
        // near ground.
        let mut j = empty_job();
        j.width = 342;
        j.verts = vec![
            // the sky (z == w: the engine's depth conversion tops out at ~0.99997, at least SKY)
            vert(43.0, 0.0, 0x7FFF, 0x7FFF, 0.0, 31.0, 0.0),
            vert(299.0, 0.0, 0x7FFF, 0x7FFF, 0.0, 31.0, 0.0),
            vert(299.0, 80.0, 0x7FFF, 0x7FFF, 0.0, 31.0, 0.0),
            vert(43.0, 80.0, 0x7FFF, 0x7FFF, 0.0, 31.0, 0.0),
            vert(43.0, 80.0, 0x7FF0, 0x8000, 31.0, 0.0, 0.0),
            vert(299.0, 80.0, 0x7FF0, 0x8000, 31.0, 0.0, 0.0),
            vert(299.0, 192.0, 0x7FF0, 0x8000, 31.0, 0.0, 0.0),
            vert(43.0, 192.0, 0x7FF0, 0x8000, 31.0, 0.0, 0.0),
        ];
        j.polys = vec![
            GxPoly { first: 0, n: 4, attr: 31 << 16 | 5 << 24, tex: 0, pal: 0, translucent: false, sortkey: 0 },
            GxPoly { first: 4, n: 4, attr: 31 << 16 | 7 << 24, tex: 0, pal: 0, translucent: false, sortkey: 0 },
        ];
        let (want, _, _) = super::super::render3d::render_maps(&j, 1);
        for y in [0, 40, 79] {
            assert_eq!(want[y * 342 + 10], want[y * 342 + 43], "a row with sky did not fill from its own sky column");
            assert_eq!(want[y * 342 + 341], want[y * 342 + 298]);
        }
        for y in [80, 120, 191] {
            assert_eq!(want[y * 342 + 10], want[79 * 342 + 43], "a row without sky did not fill from the nearest sky row");
            assert_eq!(want[y * 342 + 341], want[79 * 342 + 298]);
        }
        // the sides under the sky's end hold the sky's green, not the ground's red
        assert_eq!(want[120 * 342 + 10] & 0x3FFFF, 63 << 6);
        both_maps(&j, 1);
        both_maps(&j, 2);
    }
    /// Supersampling (этап 6.0): the GPU draws the frame k times as large and averages like the
    /// software engine, so the two agree word for word at every factor and scale; a triangle's
    /// slanted edge comes out partly covered, and with the outline on too.
    #[test]
    fn supersampling_matches_the_software_engine() {
        let tri = |msaa: u8| {
            let mut j = job_with(
                vec![vert(0.0, 0.0, 0, 0x1000, 31.0, 31.0, 31.0), vert(8.6, 0.0, 0, 0x1000, 31.0, 31.0, 31.0), vert(0.0, 8.6, 0, 0x1000, 31.0, 31.0, 31.0)],
                3,
                31 << 16,
            );
            j.clear_color = 0;
            j.extras.msaa = msaa;
            j
        };
        for (msaa, scale) in [(2, 1), (4, 1), (2, 2)] {
            let j = tri(msaa);
            let Some(px) = both(&j, scale) else { return };
            assert_eq!(px.len(), 256 * 192 * scale * scale, "the picture keeps its size");
        }
        // partly covered edge pixel: 1 of 4 samples, alpha 8 of 31, like the software test
        let Some(px) = both(&tri(2), 1) else { return };
        assert_eq!(px[3 * 256 + 5] >> 18 & 31, 8);
        // the outline and a wide view at the same time
        let mut j = tri(2);
        j.extras.outline = 0.5;
        j.disp3dcnt |= 1 << 5;
        j.width = 320;
        both(&j, 1);
    }

    /// A timing, not a check (этап 6.1.1): the GPU's and the software engine's time per frame on the
    /// dump `SUNPETAL_BENCH_DUMP` names, at scales 1-2 and supersampling off, 2 and 4.
    /// `SUNPETAL_BENCH_DUMP=... cargo test --release --features gpu --lib bench_supersampling -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_supersampling() {
        let Some(path) = std::env::var_os("SUNPETAL_BENCH_DUMP") else { return };
        let mut d = super::super::jobdump::read(std::path::Path::new(&path)).unwrap();
        for scale in [1usize, 2] {
            for msaa in [0u8, 2, 4] {
                d.job.extras.msaa = msaa;
                let job = &d.job;
                let time = |f: &dyn Fn()| {
                    f(); // warm-up: pipelines, the texture atlas
                    let t = std::time::Instant::now();
                    for _ in 0..5 {
                        f();
                    }
                    t.elapsed().as_secs_f64() * 200.0
                };
                let gpu = time(&|| drop(render(job, scale).unwrap()));
                let soft = time(&|| drop(render3d::render(job, scale)));
                println!("scale {scale} msaa {msaa}: gpu {gpu:.1} ms, software {soft:.1} ms");
            }
        }
    }

    /// The pooled attachments (этап 9.1.1-9.1.3): the frames of the dumps drawn one after the other in
    /// one order, in the reverse one and with the scale changing 1, 2, 1 must all be the software engine's
    /// words (a frame never inherits the last one's contents), and the pool stays within its limits.
    #[test]
    fn pooled_attachments_do_not_leak_between_frames() {
        let Some(dir) = std::env::var_os("SUNPETAL_JOB_DUMPS") else { return };
        let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| Some(e.ok()?.path())).filter(|p| p.extension().is_some_and(|e| e == "spjob")).collect();
        files.sort();
        let jobs: Vec<_> = files.iter().map(|f| super::super::jobdump::read(f).unwrap()).collect();
        let mut ran = 0;
        for order in [(0..jobs.len()).collect::<Vec<_>>(), (0..jobs.len()).rev().collect::<Vec<_>>()] {
            for scale_step in [1usize, 2, 1] {
                for &i in &order {
                    let job = stage7w_job(&jobs[i].job);
                    if both(&job, scale_step).is_some() {
                        ran += 1;
                    }
                    assert!(pool_len() <= POOL_MAX_ENTRIES);
                }
            }
        }
        println!("{ran} frames in two orders and three scales, pool {} entries", pool_len());
    }

    /// The supersampling on real frames (этап 6.2.1): every `*.spjob` in `SUNPETAL_JOB_DUMPS`,
    /// drawn with msaa 2 and 4, must come out word for word like the software engine's.
    /// `SUNPETAL_JOB_DUMPS=$PWD/build/gpu-parity/dumps cargo test --release --features gpu --lib supersampling_on_dumps -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn supersampling_on_dumps() {
        let Some(dir) = std::env::var_os("SUNPETAL_JOB_DUMPS") else { return };
        let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| Some(e.ok()?.path())).filter(|p| p.extension().is_some_and(|e| e == "spjob")).collect();
        files.sort();
        for f in files {
            let d = super::super::jobdump::read(&f).unwrap();
            for msaa in [2u8, 4] {
                let mut job = stage7w_job(&d.job);
                job.extras.msaa = msaa;
                both(&job, d.scale);
                println!("{} msaa {msaa}: identical", f.display());
            }
        }
    }

}
