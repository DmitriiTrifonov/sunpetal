//! The 3D rendering engine on the GPU (wgpu): a second backend behind the same job stream the
//! software renderer (`render3d`) draws, this one made for speed at large scales. Этап 2 of
//! `docs/GPU-BACKEND.md`: opaque polygons with textures — the exact rasterizer rules of the
//! software engine (fan triangulation, the top-left ownership of edges, perspective-correct
//! colour and texture coordinates, z/w depth, the less-than depth test) in WGSL, the picture read
//! back as the same words. The textures a frame uses are decoded on the CPU (by the same code the
//! texture packs dump with) into a picture atlas (`Rgba8Uint`), and the shader samples it exactly
//! as the software engine does: nearest, or the alpha-weighted bilinear (a transparent texel's
//! colour never bleeds into its neighbours), wrapped by the texture's flags, blended by the
//! polygon's mode (modulate/decal/toon-highlight). The parity harness (`gpu_parity_check`) scores
//! it against the software renderer on the same filtered job (`stage2_job`).
//!
//! The rules the software engine implements that the GPU one still does not: translucent polygons
//! (and the texture formats only they use, A3I5 and A5I3), shadow polygons, wireframe polygons,
//! fog, edge marking, the rear plane's bitmap, the widened view, the outline and the shading,
//! MSAA. `stage2_job` takes those out of a job, so `render` need not decide per polygon; on a
//! full job it would miss pixels, which stage 3+ will fix.

use super::render3d::{self, GxVertex, Image, Job, Mem, TexKey};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use wgpu::util::DeviceExt;

/// A wgpu device of the process, made on first use and kept (the parity harness renders many
/// frames; the game loop will too).
struct Ctx {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
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
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("render3d_gpu"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    });
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
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 8,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                }],
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::TextureFormat::R32Uint.into())],
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
    Ok(Ctx { device, queue, pipeline, bgl })
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

/// The job the stage-2 GPU renderer draws: the polygons it can (opaque, textured or not, not a
/// shadow, not wireframe), and the frame-wide bits it does not do yet (edge marking, fog, the rear
/// plane, the widened view, the extras) turned off. Rendered on the software and the GPU engine,
/// the job gives the same picture to compare (`gpu_parity_check`).
pub(super) fn stage2_job(job: &Job) -> Job {
    let polys = job
        .polys
        .iter()
        .filter(|p| !p.translucent && (p.attr >> 4) & 3 != 3 && (p.attr >> 16) & 31 != 0)
        .copied()
        .collect();
    Job {
        verts: job.verts.clone(),
        polys,
        wbuffer: job.wbuffer,
        disp3dcnt: job.disp3dcnt & !(0x20 | 0x80 | 0x4000),
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
/// below would keep) and the `Tri` records the fragment shader tests with.
fn build(job: &Job, s: usize, regions: &HashMap<(u32, u32), TexInfo>) -> (Vec<u8>, Vec<u8>, u32) {
    let sc = s as f32;
    let texture_on = job.disp3dcnt & 1 != 0;
    let mut vpos = Vec::new();
    let mut tris = Vec::new();
    let push = |v: &mut Vec<u8>, f: f32| v.extend_from_slice(&f.to_le_bytes());
    for p in &job.polys {
        let vs = &job.verts[p.first as usize..(p.first + p.n) as usize];
        let mode = (p.attr >> 4) & 3;
        let textured = texture_on && (p.tex >> 26) & 7 != 0;
        let texinfo = textured.then(|| regions.get(&(p.tex, p.pal)).copied()).flatten();
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
            push(&mut tris, pa.0);
            push(&mut tris, pa.1);
            push(&mut tris, pb.0);
            push(&mut tris, pb.1);
            push(&mut tris, pc.0);
            push(&mut tris, pc.1);
            push(&mut tris, area);
            let o = (owns(pb, pc) as u32) | (owns(pc, pa) as u32) << 1 | (owns(pa, pb) as u32) << 2;
            tris.extend_from_slice(&o.to_le_bytes());
            tris.extend_from_slice(&mode.to_le_bytes());
            pack(a, &mut tris);
            pack(b, &mut tris);
            pack(c, &mut tris);
            // the texture: where its picture sits in the atlas, and its coordinates (an all-zero
            // record with `w` 0 is no texture)
            let ti = texinfo.unwrap_or(TexInfo { ux: 0, uy: 0, w: 0, h: 0, ow: 0.0, oh: 0.0, flags: 0 });
            for u in [ti.ux, ti.uy, ti.w, ti.h] {
                tris.extend_from_slice(&u.to_le_bytes());
            }
            push(&mut tris, ti.ow);
            push(&mut tris, ti.oh);
            tris.extend_from_slice(&ti.flags.to_le_bytes());
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
                push(&mut vpos, v.0 as f32);
                push(&mut vpos, v.1 as f32);
            }
        }
    }
    let nvert = (vpos.len() / 8) as u32;
    (vpos, tris, nvert)
}

/// Draws a job as stage 2 can and reads the picture back as the same words the software renderer
/// returns, with its `notes` mask (only the AA bit can be set; the rest of the software engine's
/// notes are about things the job has none of).
pub(super) fn render(job: &Job, scale: usize) -> Result<(Vec<u32>, u32), String> {
    let c = ctx()?;
    let s = scale.max(1);
    let (w, h) = (job.width * s, 192 * s);
    let mut notes = 0u32;
    if job.disp3dcnt & 0x10 != 0 {
        notes |= 1 << 1;
    }
    // the uniforms: sizes, the clear word, the modes, the toon table
    let (cr, cg, cb) = render3d::rgb555((job.clear_color & 0x7FFF) as u16);
    let calpha = (job.clear_color >> 16) & 31;
    let clear_word = if calpha == 0 { 0 } else { cr | cg << 6 | cb << 12 | calpha << 18 };
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
    let device = &c.device;
    let max_dim = device.limits().max_texture_dimension_2d;
    if atlas_w > max_dim || atlas_h > max_dim {
        return Err(format!("the frame's textures make a {atlas_w}x{atlas_h} atlas, over the device's {max_dim}"));
    }
    let atlas = device.create_texture(&wgpu::TextureDescriptor {
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
    let (vpos, tridata, nvert) = build(job, s, &regions);
    if nvert == 0 {
        // no triangles: the clear plane alone
        return Ok((vec![clear_word; w * h], notes));
    }
    let mut uni = Vec::with_capacity(40 * 4);
    for u in [
        w as u32,
        h as u32,
        clear_word,
        job.wbuffer as u32,
        (job.disp3dcnt & 4 != 0) as u32,
        job.alpha_ref as u32,
        (job.disp3dcnt & 2 != 0) as u32,
        matches!(job.extras.filter, super::TexFilter::Bilinear) as u32,
    ] {
        uni.extend_from_slice(&u.to_le_bytes());
    }
    for t in job.toon {
        uni.extend_from_slice(&(t as u32).to_le_bytes());
    }
    let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("3d verts"),
        contents: &vpos,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
    });
    let tb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("3d tris"),
        contents: &tridata,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });
    let ub = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("3d uniforms"),
        contents: &uni,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let av = atlas.create_view(&Default::default());
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("3d"),
        layout: &c.bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: tb.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&av) },
        ],
    });
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("3d words"),
        size: wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Uint,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("3d depth"),
        size: wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let cv = color.create_view(&Default::default());
        let dv = depth.create_view(&Default::default());
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3d"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &cv,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: clear_word as f64, g: 0.0, b: 0.0, a: 0.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &dv,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(render3d::clear_depth(job.clear_depth) as f32),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(&c.pipeline);
        rp.set_bind_group(0, &bind, &[]);
        rp.set_vertex_buffer(0, vb.slice(..));
        rp.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
        if nvert > 0 {
            rp.draw(0..nvert, 0..1);
        }
    }
    // the picture back: copy to a mappable buffer and read the rows
    let row = ((w as u32 * 4).div_ceil(256) * 256) as u64;
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("3d readback"),
        size: row * h as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: &color, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row as u32), rows_per_image: Some(h as u32) },
        },
        wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
    );
    c.queue.submit([enc.finish()]);
    let slice = buf.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = c.device.poll(wgpu::PollType::wait_indefinitely());
    let mut words = Vec::with_capacity(w * h);
    if let Ok(map) = slice.get_mapped_range() {
        for y in 0..h {
            let base = y * row as usize;
            for x in 0..w {
                let p = base + x * 4;
                words.push(u32::from_le_bytes([map[p], map[p + 1], map[p + 2], map[p + 3]]));
            }
        }
    } else {
        return Err("the 3D picture could not be read back".into());
    }
    buf.unmap();
    Ok((words, notes))
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
  // the toon table, four 15-bit colours per vec4 (uniform arrays need a stride of 16)
  toon: array<vec4u, 8>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<storage, read> tris: array<Tri>;
@group(0) @binding(2) var atlas: texture_2d<u32>;

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
  @builtin(frag_depth) depth: f32,
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

@fragment
fn fs(in: VOut) -> FOut {
  let t = tris[in.tri_i];
  // the pixel centre, exactly as the software engine computes it (x + 0.5, y + 0.5): the
  // rasterizer's interpolated position may be an ulp off, which the exact engine never is
  let pt = vec2f(floor(in.clip.x) + 0.5, floor(in.clip.y) + 0.5);
  let a = vec2f(t.ax, t.ay);
  let b = vec2f(t.bx, t.by);
  let c = vec2f(t.cx, t.cy);
  let w0 = edge(b, c, pt);
  let w1 = edge(c, a, pt);
  let w2 = edge(a, b, pt);
  // the software engine's exact inside test: strictly in, or on an edge the polygon owns
  let in0 = w0 > 0.0 || (w0 == 0.0 && (t.owns & 1u) != 0u);
  let in1 = w1 > 0.0 || (w1 == 0.0 && (t.owns & 2u) != 0u);
  let in2 = w2 > 0.0 || (w2 == 0.0 && (t.owns & 4u) != 0u);
  if (!(in0 && in1 && in2)) {
    discard;
  }
  let l0 = div(w0, t.area);
  let l1 = div(w1, t.area);
  let l2 = div(w2, t.area);
  var depth: f32;
  if (g.wbuffer != 0u) {
    let den = (fma(l0, t.iw0, 0.0) + fma(l1, t.iw1, 0.0)) + fma(l2, t.iw2, 0.0);
    depth = div(div(1.0, den), 16777215.0);
  } else {
    depth = (fma(l0, t.d0, 0.0) + fma(l1, t.d1, 0.0)) + fma(l2, t.d2, 0.0);
  }
  // the colour, perspective-correct, exactly as the software engine weights it (the sums keep
  // the engine's order of operations)
  let p0 = l0 * t.iw0;
  let p1 = l1 * t.iw1;
  let p2 = l2 * t.iw2;
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
  var oa: u32 = 31u;
  if (t.tw != 0u) {
    let c = select(tex_nearest(t, s_, tt), tex_bilinear(t, s_, tt), g.bilinear != 0u);
    let tr6 = c.x;
    let tg6 = c.y;
    let tb6 = c.z;
    let ta5 = c.w;
    if (t.mode == 1u) {
      // decal: the texture over the vertex colour by its alpha (the polygon's own alpha stays 31)
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
      // modulate (and the toon): the colours and the alpha multiply; an opaque polygon's alpha
      // is 31, so the texel's alpha is all-or-nothing
      r6 = ((tr6 + 1u) * (r6 + 1u) - 1u) / 64u;
      g6 = ((tg6 + 1u) * (g6 + 1u) - 1u) / 64u;
      b6 = ((tb6 + 1u) * (b6 + 1u) - 1u) / 64u;
      oa = ((ta5 + 1u) * 32u - 1u) / 32u;
    }
  }
  if (t.mode == 2u && g.highlight != 0u) {
    // the table's colour over the textured picture
    r6 = min(r6 + toon.x, 63u);
    g6 = min(g6 + toon.y, 63u);
    b6 = min(b6 + toon.z, 63u);
  }
  // a transparent texel is gone; the alpha test may take more (an opaque polygon's alpha here is
  // either 0 or 31, so `oa <= alpha_ref` only bites when the test refuses 31 itself)
  if (oa == 0u || (g.alpha_test != 0u && oa <= g.alpha_ref)) {
    discard;
  }
  var o: FOut;
  o.word = vec4u(r6 | g6 << 6u | b6 << 12u | 31u << 18u, 0u, 0u, 0u);
  o.depth = depth;
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
                    i % (256 * scale),
                    i / (256 * scale),
                    a,
                    b
                ));
            }
            panic!("{msg}");
        }
        Some(want)
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
}
