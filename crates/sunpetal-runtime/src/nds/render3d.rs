//! The rendering engine, in software: a frame's polygons, already transformed, lit and clipped by
//! the geometry engine (`gx`), with the 3D registers and the texture memory as they were when the
//! DS started drawing it, to a 256x192 picture.
//!
//! Done: perspective-correct colours and texture coordinates, the depth buffer (z or w), less-than
//! and "equal" depth tests, all seven texture formats (nearest filtering, repeat, flip, clamp),
//! modulation, decal, toon and highlight shading, polygon alpha with blending over what is there,
//! alpha test, wireframe polygons, the clear colour, depth and polygon ID.
//! Not done (named in the log once each): fog, edge marking, anti-aliasing, shadow polygons, the
//! rear plane's bitmap.
//!
//! A pixel of the picture is RGB666 (r bits 0-5, g 6-11, b 12-17) and alpha in bits 18-22 (0: no
//! pixel, the 2D layers under show), as the 2D compositor takes it.

/// A polygon's vertex after clipping.
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct GxVertex {
    /// Clip coordinates (20.12), after clipping.
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub w: i32,
    /// The screen position, exact (pixels, y down).
    pub sx: f32,
    pub sy: f32,
    /// ... as the DS rounds it.
    pub ix: i16,
    pub iy: i16,
    /// The colour, 0-31 (fractions where clipping interpolated).
    pub r: f32,
    pub g: f32,
    pub b: f32,
    /// Texture coordinates, in texels.
    pub s: f32,
    pub t: f32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct GxPoly {
    /// Its vertices in the frame's list.
    pub first: u32,
    pub n: u32,
    /// POLYGON_ATTR.
    pub attr: u32,
    /// TEXIMAGE_PARAM, PLTT_BASE.
    pub tex: u32,
    pub pal: u32,
    pub translucent: bool,
    /// Bottom y << 8 | top y.
    pub sortkey: u32,
}

pub(super) struct Job {
    pub verts: Vec<GxVertex>,
    /// In the order the rendering engine draws them.
    pub polys: Vec<GxPoly>,
    /// SWAP_BUFFERS bit 1: depth is w.
    pub wbuffer: bool,
    pub disp3dcnt: u16,
    /// CLEAR_COLOR: colour, fog bit, alpha, polygon ID.
    pub clear_color: u32,
    pub clear_depth: u16,
    pub alpha_ref: u8,
    pub edge: [u16; 8],
    pub fog_color: u32,
    pub fog_offset: u16,
    pub fog_table: [u8; 32],
    pub toon: [u16; 32],
    /// The texture slots, 512 KiB.
    pub tex: Vec<u8>,
    /// The texture-palette slots, 96 KiB.
    pub texpal: Vec<u8>,
}

/// What `render` leaves out, by bit of the mask it returns.
pub(super) const NOTE: [&str; 5] = [
    "3D: fog is not drawn",
    "3D: edge marking and anti-aliasing are not drawn",
    "3D: shadow polygons are drawn as modulated ones",
    "3D: the rear plane's bitmap is not drawn (the clear colour is)",
    "3D: a polygon with a texture format 0 or no texture mapping is drawn untextured",
];

fn c5to6(c: u32) -> u32 {
    if c != 0 { c * 2 + 1 } else { 0 }
}

/// A 15-bit palette colour as RGB666.
fn rgb555(c: u16) -> (u32, u32, u32) {
    (c5to6(c as u32 & 31), c5to6(c as u32 >> 5 & 31), c5to6(c as u32 >> 10 & 31))
}

fn pal_color(job: &Job, off: usize) -> u16 {
    if off + 1 < job.texpal.len() { u16::from_le_bytes([job.texpal[off], job.texpal[off + 1]]) } else { 0 }
}

fn tex8(job: &Job, off: usize) -> u8 {
    job.tex.get(off & 0x7FFFF).copied().unwrap_or(0)
}

/// One texel: (r, g, b) in 6 bits and alpha in 5.
fn sample(job: &Job, tex: u32, pal: u32, s: f32, t: f32) -> (u32, u32, u32, u32) {
    let fmt = (tex >> 26) & 7;
    let w = 8i32 << ((tex >> 20) & 7);
    let h = 8i32 << ((tex >> 23) & 7);
    let wrap = |v: i32, size: i32, repeat: bool, flip: bool| -> i32 {
        if repeat {
            let m = v.rem_euclid(size);
            if flip && v.div_euclid(size) & 1 != 0 { size - 1 - m } else { m }
        } else {
            v.clamp(0, size - 1)
        }
    };
    let x = wrap(s.floor() as i32, w, tex & 0x10000 != 0, tex & 0x40000 != 0) as usize;
    let y = wrap(t.floor() as i32, h, tex & 0x20000 != 0, tex & 0x80000 != 0) as usize;
    let base = ((tex & 0xFFFF) as usize) * 8;
    let (wu, c0t) = (w as usize, tex & 0x2000_0000 != 0);
    let pal16 = pal as usize * 16;
    let opaque = |c: u16| {
        let (r, g, b) = rgb555(c);
        (r, g, b, 31)
    };
    match fmt {
        1 => {
            let v = tex8(job, base + y * wu + x) as u32;
            let (r, g, b) = rgb555(pal_color(job, pal16 + (v & 31) as usize * 2));
            let a3 = v >> 5;
            (r, g, b, a3 * 4 + a3 / 2)
        }
        2 => {
            let v = tex8(job, base + (y * wu + x) / 4) as u32 >> (2 * (x & 3)) & 3;
            if v == 0 && c0t {
                return (0, 0, 0, 0);
            }
            opaque(pal_color(job, pal as usize * 8 + v as usize * 2))
        }
        3 => {
            let b = tex8(job, base + (y * wu + x) / 2) as u32;
            let v = if x & 1 != 0 { b >> 4 } else { b & 15 };
            if v == 0 && c0t {
                return (0, 0, 0, 0);
            }
            opaque(pal_color(job, pal16 + v as usize * 2))
        }
        4 => {
            let v = tex8(job, base + y * wu + x) as usize;
            if v == 0 && c0t {
                return (0, 0, 0, 0);
            }
            opaque(pal_color(job, pal16 + v * 2))
        }
        5 => {
            let tile = (y >> 2) * (wu >> 2) + (x >> 2);
            let data = base + tile * 4 + (y & 3);
            let idx = (tex8(job, data) as u32 >> (2 * (x & 3))) & 3;
            let info_at = if base < 0x40000 { 0x20000 + (base >> 1) + tile * 2 } else { 0x30000 + ((base - 0x40000) >> 1) + tile * 2 };
            let info = u16::from_le_bytes([tex8(job, info_at), tex8(job, info_at + 1)]);
            let po = pal16 + (info & 0x3FFF) as usize * 4;
            let col = |i: usize| rgb555(pal_color(job, po + 2 * i));
            let (c0, c1) = (col(0), col(1));
            let avg = |a: (u32, u32, u32), b: (u32, u32, u32), wa: u32, wb: u32| ((a.0 * wa + b.0 * wb) / 8, (a.1 * wa + b.1 * wb) / 8, (a.2 * wa + b.2 * wb) / 8);
            let c = match (info >> 14, idx) {
                (_, 0) => c0,
                (_, 1) => c1,
                (0, 2) | (2, 2) => col(2),
                (0, _) | (1, _) if idx == 3 => return (0, 0, 0, 0),
                (1, _) => avg(c0, c1, 4, 4),
                (2, _) => col(3),
                (_, 2) => avg(c0, c1, 5, 3),
                _ => avg(c0, c1, 3, 5),
            };
            (c.0, c.1, c.2, 31)
        }
        6 => {
            let v = tex8(job, base + y * wu + x) as u32;
            let (r, g, b) = rgb555(pal_color(job, pal16 + (v & 7) as usize * 2));
            (r, g, b, v >> 3)
        }
        7 => {
            let o = base + (y * wu + x) * 2;
            let c = u16::from_le_bytes([tex8(job, o), tex8(job, o + 1)]);
            let (r, g, b) = rgb555(c);
            (r, g, b, if c & 0x8000 != 0 { 31 } else { 0 })
        }
        _ => (63, 63, 63, 31),
    }
}

/// CLEAR_DEPTH's 15 bits as the depth buffer holds them (0-1 of 24 bits).
fn clear_depth(d: u16) -> f32 {
    let d = d as u32;
    (d * 0x200 + ((d + 1) >> 15) * 0x1FF) as f32 / 16_777_215.0
}

struct Target {
    color: Vec<u32>,
    alpha: Vec<u8>,
    depth: Vec<f32>,
    id: Vec<u8>,
}

fn edge(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> f32 {
    (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
}

/// Draws a frame. Returns the picture and the mask of what it left out (`NOTE`).
pub(super) fn render(job: &Job) -> (Vec<u32>, u32) {
    let mut notes = 0u32;
    let cc = job.clear_color;
    let (cr, cg, cb) = rgb555((cc & 0x7FFF) as u16);
    let calpha = ((cc >> 16) & 31) as u8;
    let cid = ((cc >> 24) & 63) as u8;
    let n = 256 * 192;
    let mut t = Target {
        color: vec![cr | cg << 6 | cb << 12; n],
        alpha: vec![calpha; n],
        depth: vec![clear_depth(job.clear_depth); n],
        id: vec![cid; n],
    };
    if job.disp3dcnt & 0x4000 != 0 {
        notes |= 1 << 3;
    }
    if job.disp3dcnt & 0x80 != 0 {
        notes |= 1;
    }
    if job.disp3dcnt & 0x30 != 0 {
        notes |= 1 << 1;
    }
    let probe: Option<(usize, usize)> = std::env::var("SUNPETAL_3D_PIXEL").ok().and_then(|v| { let mut it = v.split(','); Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?)) });
    let texture_on = job.disp3dcnt & 1 != 0;
    let highlight = job.disp3dcnt & 2 != 0;
    let alpha_test = job.disp3dcnt & 4 != 0;
    let blend_on = job.disp3dcnt & 8 != 0;
    for p in &job.polys {
        let vs = &job.verts[p.first as usize..(p.first + p.n) as usize];
        let attr = p.attr;
        let polyalpha = ((attr >> 16) & 31) as u32;
        let pid = ((attr >> 24) & 63) as u8;
        let mode = (attr >> 4) & 3;
        if mode == 3 {
            notes |= 1 << 2;
        }
        let fmt = (p.tex >> 26) & 7;
        let textured = texture_on && fmt != 0;
        if texture_on && fmt == 0 {
            notes |= 1 << 4;
        }
        let eq_depth = attr & 0x4000 != 0;
        let write_trans_depth = attr & 0x800 != 0;
        let wire = polyalpha == 0;
        // fill one pixel
        let put = |t: &mut Target, x: usize, y: usize, depth: f32, vr: f32, vg: f32, vb: f32, s: f32, tt: f32| {
            let i = y * 256 + x;
            let d = t.depth[i];
            let tol = 0x200 as f32 / 16_777_215.0;
            if eq_depth { if (depth - d).abs() > tol { return; } } else if depth >= d { return; }
            let to6 = |c: f32| -> u32 { (c * 63.0 / 31.0).round().clamp(0.0, 63.0) as u32 };
            let (mut r, mut g, mut b) = (to6(vr), to6(vg), to6(vb));
            let mut va = if wire { 31 } else { polyalpha };
            if mode == 2 {
                // toon (and highlight): the vertex colour's red picks the table's colour
                let tc = job.toon[(r >> 1).min(31) as usize];
                let (tr, tg, tb) = rgb555(tc);
                if highlight {
                    let gray = r;
                    r = gray;
                    g = gray;
                    b = gray;
                } else {
                    r = tr;
                    g = tg;
                    b = tb;
                }
            }
            let (mut or, mut og, mut ob, mut oa) = (r, g, b, va);
            if textured {
                let (tr, tg, tb, ta) = sample(job, p.tex, p.pal, s, tt);
                if mode == 1 {
                    // decal: the texture over the vertex colour by its alpha
                    if ta == 0 {
                        // the vertex colour
                    } else if ta == 31 {
                        or = tr;
                        og = tg;
                        ob = tb;
                    } else {
                        or = (tr * ta + r * (31 - ta)) / 31;
                        og = (tg * ta + g * (31 - ta)) / 31;
                        ob = (tb * ta + b * (31 - ta)) / 31;
                    }
                    oa = va;
                } else {
                    or = ((tr + 1) * (r + 1) - 1) / 64;
                    og = ((tg + 1) * (g + 1) - 1) / 64;
                    ob = ((tb + 1) * (b + 1) - 1) / 64;
                    oa = ((ta + 1) * (va + 1) - 1) / 32;
                }
                va = oa;
            }
            if mode == 2 && highlight {
                let (tr, tg, tb) = rgb555(job.toon[(r >> 1).min(31) as usize]);
                or = (or + tr).min(63);
                og = (og + tg).min(63);
                ob = (ob + tb).min(63);
            }
            if !blend_on && oa > 0 {
                oa = 31;
            }
            if oa == 0 || (alpha_test && oa <= job.alpha_ref as u32) {
                return;
            }
            if probe == Some((x, y)) {
                for ty in 0..8 {
                    let row: Vec<String> = (0..8).map(|tx| { let c = sample(job, p.tex, p.pal, tx as f32 * 8.0 + 0.5, ty as f32 * 8.0 + 0.5); format!("{:02}.{:02}.{:02}/{}", c.0, c.1, c.2, c.3) }).collect();
                    eprintln!("  texel row {}: {}", ty * 8, row.join(" "));
                }
                eprintln!("probe {},{}: poly attr {:08X} tex {:08X} pal {:X} st {:.2},{:.2} -> rgb {} {} {} a {} (vertex {} {} {} a {})", x, y, attr, p.tex, p.pal, s, tt, or, og, ob, oa, r, g, b, va);
            }
            if oa >= 31 {
                t.color[i] = or | og << 6 | ob << 12;
                t.alpha[i] = 31;
                t.depth[i] = depth;
                t.id[i] = pid;
            } else {
                if t.id[i] == pid && t.alpha[i] > 0 && t.alpha[i] < 31 {
                    return; // translucent polygons do not draw over their own ID
                }
                if t.alpha[i] == 0 {
                    t.color[i] = or | og << 6 | ob << 12;
                    t.alpha[i] = oa as u8;
                } else {
                    let d = t.color[i];
                    let mix = |s: u32, dd: u32| (s * (oa + 1) + dd * (31 - oa)) / 32;
                    t.color[i] = mix(or, d & 63) | mix(og, d >> 6 & 63) << 6 | mix(ob, d >> 12 & 63) << 12;
                    t.alpha[i] = t.alpha[i].max(oa as u8);
                }
                if write_trans_depth {
                    t.depth[i] = depth;
                }
                t.id[i] = pid;
            }
        };
        let depth_of = |v: &GxVertex| -> f32 {
            let w = v.w.max(1) as f32;
            // the DS's 24-bit depth: ((z * 0x4000 / w) + 0x3FFF) * 0x200 (a polygon on the far plane
            // is nearer than the cleared buffer)
            if job.wbuffer { w / 16_777_215.0 } else { ((v.z as f32 / w) * 16384.0 + 16383.0) * 512.0 / 16_777_215.0 }
        };
        if wire {
            // wireframe: the polygon's edges in its vertex colours
            for k in 0..vs.len() {
                let (a, b) = (&vs[k], &vs[(k + 1) % vs.len()]);
                let (dx, dy) = (b.sx - a.sx, b.sy - a.sy);
                let steps = dx.abs().max(dy.abs()).ceil().max(1.0) as i32;
                for s in 0..=steps {
                    let f = s as f32 / steps as f32;
                    let (x, y) = ((a.sx + dx * f).floor() as i32, (a.sy + dy * f).floor() as i32);
                    if !(0..256).contains(&x) || !(0..192).contains(&y) {
                        continue;
                    }
                    let d = depth_of(a) + (depth_of(b) - depth_of(a)) * f;
                    let l = |p: f32, q: f32| p + (q - p) * f;
                    put(&mut t, x as usize, y as usize, d, l(a.r, b.r), l(a.g, b.g), l(a.b, b.b), l(a.s, b.s), l(a.t, b.t));
                }
            }
            continue;
        }
        // a convex polygon as a fan of triangles
        for k in 1..vs.len() - 1 {
            let (a, mut b, mut c) = (&vs[0], &vs[k], &vs[k + 1]);
            let area = edge((a.sx, a.sy), (b.sx, b.sy), (c.sx, c.sy));
            if area == 0.0 {
                continue;
            }
            if area < 0.0 {
                std::mem::swap(&mut b, &mut c);
            }
            let area = area.abs();
            let (pa, pb, pc) = ((a.sx, a.sy), (b.sx, b.sy), (c.sx, c.sy));
            let x0 = pa.0.min(pb.0).min(pc.0).floor().max(0.0) as usize;
            let x1 = (pa.0.max(pb.0).max(pc.0).ceil().min(255.0)) as usize;
            let y0 = pa.1.min(pb.1).min(pc.1).floor().max(0.0) as usize;
            let y1 = (pa.1.max(pb.1).max(pc.1).ceil().min(191.0)) as usize;
            let iw = |v: &GxVertex| 1.0 / (v.w.max(1) as f32);
            let (ia, ib, ic) = (iw(a), iw(b), iw(c));
            let (da, db, dc) = (depth_of(a), depth_of(b), depth_of(c));
            // top-left rule: a pixel on an edge belongs to the polygon on one side only
            let owns = |p: (f32, f32), q: (f32, f32)| -> bool { let dy = q.1 - p.1; let dx = q.0 - p.0; dy < 0.0 || (dy == 0.0 && dx > 0.0) };
            let (oa_, ob_, oc_) = (owns(pb, pc), owns(pc, pa), owns(pa, pb));
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let pt = (x as f32 + 0.5, y as f32 + 0.5);
                    let (w0, w1, w2) = (edge(pb, pc, pt), edge(pc, pa, pt), edge(pa, pb, pt));
                    let inside = |w: f32, own: bool| w > 0.0 || (w == 0.0 && own);
                    if !(inside(w0, oa_) && inside(w1, ob_) && inside(w2, oc_)) {
                        continue;
                    }
                    let (l0, l1, l2) = (w0 / area, w1 / area, w2 / area);
                    let depth = if job.wbuffer {
                        let den = l0 * ia + l1 * ib + l2 * ic;
                        (1.0 / den) / 16_777_215.0
                    } else {
                        l0 * da + l1 * db + l2 * dc
                    };
                    let (p0, p1, p2) = (l0 * ia, l1 * ib, l2 * ic);
                    let den = p0 + p1 + p2;
                    let (q0, q1, q2) = (p0 / den, p1 / den, p2 / den);
                    let at = |fa: f32, fb: f32, fc: f32| q0 * fa + q1 * fb + q2 * fc;
                    put(&mut t, x, y, depth, at(a.r, b.r, c.r), at(a.g, b.g, c.g), at(a.b, b.b, c.b), at(a.s, b.s, c.s), at(a.t, b.t, c.t));
                }
            }
        }
    }
    let out = (0..n).map(|i| if t.alpha[i] == 0 { 0 } else { t.color[i] | (t.alpha[i] as u32) << 18 }).collect();
    (out, notes)
}
