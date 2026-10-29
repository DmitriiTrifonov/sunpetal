//! The 3D rendering engine on the GPU (wgpu): a second backend behind the same job stream the
//! software renderer (`render3d`) draws, this one made for speed at large scales. Этап 1 of
//! `docs/GPU-BACKEND.md`: opaque polygons without textures — the exact rasterizer rules of the
//! software engine (fan triangulation, the top-left ownership of edges, perspective-correct
//! colour, z/w depth, the less-than depth test) in WGSL, the picture read back as the same
//! words. The parity harness (`gpu_parity_check`) scores it against the software renderer on the
//! same filtered job (`stage1_job`).
//!
//! The rules the software engine implements that the GPU one still does not: textures, translucent
//! polygons, shadow polygons, wireframe polygons, fog, edge marking, the rear plane's bitmap, the
//! widened view, the outline and the shading, MSAA. `stage1_job` takes those out of a job, so
//! `render` need not decide per polygon; on a full job it would miss pixels, which stage 2+ will
//! fix.

use super::render3d::{self, GxVertex, Job};
use std::sync::OnceLock;
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

/// The job the stage-1 GPU renderer draws: the polygons it can (opaque, untextured, not a shadow,
/// not wireframe), and the frame-wide bits it does not do yet (edge marking, fog, the rear plane,
/// the widened view, the extras) turned off. Rendered on the software and the GPU engine, the job
/// gives the same picture to compare (`gpu_parity_check`).
pub(super) fn stage1_job(job: &Job) -> Job {
    let texture_on = job.disp3dcnt & 1 != 0;
    let polys = job
        .polys
        .iter()
        .filter(|p| {
            !p.translucent
                && !(texture_on && (p.tex >> 26) & 7 != 0)
                && (p.attr >> 4) & 3 != 3
                && (p.attr >> 16) & 31 != 0
        })
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
fn build(job: &Job, s: usize) -> (Vec<u8>, Vec<u8>, u32) {
    let sc = s as f32;
    let mut vpos = Vec::new();
    let mut tris = Vec::new();
    let push = |v: &mut Vec<u8>, f: f32| v.extend_from_slice(&f.to_le_bytes());
    for p in &job.polys {
        let vs = &job.verts[p.first as usize..(p.first + p.n) as usize];
        let mode = (p.attr >> 4) & 3;
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

/// Draws a job as stage 1 can and reads the picture back as the same words the software renderer
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
    let (vpos, tridata, nvert) = build(job, s);
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
        0, // the toon array sits at offset 32 (WGSL uniform arrays need a stride of 16)
    ] {
        uni.extend_from_slice(&u.to_le_bytes());
    }
    for t in job.toon {
        uni.extend_from_slice(&(t as u32).to_le_bytes());
    }
    let device = &c.device;
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
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("3d"),
        layout: &c.bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: tb.as_entire_binding() },
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
  // the toon table, four 15-bit colours per vec4 (uniform arrays need a stride of 16)
  toon: array<vec4u, 8>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<storage, read> tris: array<Tri>;

// One fan triangle: the exact vertex positions (the software engine's), its area, the ownership
// of its edges (bits 0-2), the mode, and the interpolation data of its three vertices.
struct Tri {
  ax: f32, ay: f32, bx: f32, by: f32, cx: f32, cy: f32,
  area: f32,
  owns: u32,
  mode: u32,
  iw0: f32, d0: f32, r0: f32, g0: f32, b0: f32,
  iw1: f32, d1: f32, r1: f32, g1: f32, b1: f32,
  iw2: f32, d2: f32, r2: f32, g2: f32, b2: f32,
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
  return (q.x - p.x) * (t.y - p.y) - (q.y - p.y) * (t.x - p.x);
}

// the software engine's vertex colour to RGB666: (c * 63 / 31) rounded, clamped
fn to6(c: f32) -> u32 {
  let v = round(c * 63.0 / 31.0);
  return u32(clamp(v, 0.0, 63.0));
}

// a 5-bit colour channel to 6 bits, as the software engine widens it
fn c5to6(c: u32) -> u32 {
  return select(0u, c * 2u + 1u, c != 0u);
}

@fragment
fn fs(in: VOut) -> FOut {
  let t = tris[in.tri_i];
  // the pixel centre, exactly: the fragment position is at x + 0.5, y + 0.5
  let pt = vec2f(in.clip.x, in.clip.y);
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
  let l0 = w0 / t.area;
  let l1 = w1 / t.area;
  let l2 = w2 / t.area;
  var depth: f32;
  if (g.wbuffer != 0u) {
    let den = l0 * t.iw0 + l1 * t.iw1 + l2 * t.iw2;
    depth = (1.0 / den) / 16777215.0;
  } else {
    depth = l0 * t.d0 + l1 * t.d1 + l2 * t.d2;
  }
  // the colour, perspective-correct, exactly as the software engine weights it
  let p0 = l0 * t.iw0;
  let p1 = l1 * t.iw1;
  let p2 = l2 * t.iw2;
  let den = p0 + p1 + p2;
  let q0 = p0 / den;
  let q1 = p1 / den;
  let q2 = p2 / den;
  let vr = q0 * t.r0 + q1 * t.r1 + q2 * t.r2;
  let vg = q0 * t.g0 + q1 * t.g1 + q2 * t.g2;
  let vb = q0 * t.b0 + q1 * t.b1 + q2 * t.b2;
  var r6 = to6(vr);
  var g6 = to6(vg);
  var b6 = to6(vb);
  if (t.mode == 2u) {
    // toon (and highlight): the vertex colour's red picks the table's colour
    let ri = min(r6 >> 1u, 31u);
    let tv = g.toon[ri >> 2u];
    let tc = tv[ri & 3u];
    let tr = c5to6(tc & 31u);
    let tg = c5to6(tc >> 5u & 31u);
    let tb = c5to6(tc >> 10u & 31u);
    if (g.highlight != 0u) {
      let gray = r6;
      r6 = gray;
      g6 = gray;
      b6 = gray;
      r6 = min(r6 + tr, 63u);
      g6 = min(g6 + tg, 63u);
      b6 = min(b6 + tb, 63u);
    } else {
      r6 = tr;
      g6 = tg;
      b6 = tb;
    }
  }
  // an opaque polygon the alpha test swallows whole (the software engine's `oa <= alpha_ref`)
  if (g.alpha_test != 0u && 31u <= g.alpha_ref) {
    discard;
  }
  var o: FOut;
  o.word = vec4u(r6 | g6 << 6u | b6 << 12u | 31u << 18u, 0u, 0u, 0u);
  o.depth = depth;
  return o;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::render3d::{GxPoly, Job};
    use std::collections::HashMap;
    use std::sync::Arc;

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
            tex: vec![0; 0x20000],
            texpal: vec![0; 0x10000],
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
        if let Some((i, (a, b))) = gpu.iter().zip(&want).enumerate().find(|(_, (a, b))| a != b) {
            panic!(
                "the GPU picture differs from the software one at pixel {} (x {}, y {}): {:#010x} vs {:#010x}",
                i,
                i % (256 * scale),
                i / (256 * scale),
                a,
                b
            );
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
    fn the_clear_plane_alone_is_the_clear_word() {
        let j = empty_job();
        both(&j, 1);
    }
}
