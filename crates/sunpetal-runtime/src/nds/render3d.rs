//! The rendering engine, in software: a frame's polygons, already transformed, lit and clipped by
//! the geometry engine (`gx`), with the 3D registers and the texture memory as they were when the
//! DS started drawing it, to a 256x192 picture.
//!
//! Done: perspective-correct colours and texture coordinates, the depth buffer (z or w), less-than
//! and "equal" depth tests, all seven texture formats (nearest filtering, repeat, flip, clamp),
//! modulation, decal, toon and highlight shading, polygon alpha with blending over what is there,
//! alpha test, wireframe polygons, the clear colour, depth and polygon ID.
//! Also: edge marking (a pixel next to a farther pixel of another polygon ID takes the edge
//! colour of its ID) and shadow polygons (the mask polygon, ID 0, marks where it lies behind
//! something; the others draw there).
//! Not done (named in the log once each): fog, anti-aliasing, the rear plane's bitmap.
//!
//! At `scale` the picture is `scale` times as large each way: the polygons are rasterised at
//! that resolution, the texels and the DS's depth rules unchanged.
//!
//! A pixel of the picture is RGB666 (r bits 0-5, g 6-11, b 12-17) and alpha in bits 18-22 (0: no
//! pixel, the 2D layers under show), as the 2D compositor takes it.

use std::collections::HashMap;
use std::sync::Arc;

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
    /// The picture's width in DS pixels: 256, or more when the view is widened (`Nds::wide`).
    pub width: usize,
    /// CLEAR_COLOR: colour, fog bit, alpha, polygon ID.
    pub clear_color: u32,
    pub clear_depth: u16,
    /// CLRIMAGE_OFFSET: where the rear-plane bitmap starts (x, then y << 8).
    pub clrimage_offset: u16,
    pub alpha_ref: u8,
    /// Our own outline and shading (`outline_pass`, `shade_pass`).
    pub extras: super::Extras,
    pub edge: [u16; 8],
    pub fog_color: u32,
    pub fog_offset: u16,
    pub fog_table: [u8; 32],
    pub toon: [u16; 32],
    /// The texture slots, 512 KiB.
    pub tex: Vec<u8>,
    /// The texture-palette slots, 96 KiB.
    pub texpal: Vec<u8>,
    /// The key of each (TEXIMAGE_PARAM, PLTT_BASE) the frame uses (`texpack`).
    pub keys: HashMap<(u32, u32), TexKey>,
    /// The replacement images by key.
    pub pack: Arc<super::texpack::Pack>,
}

/// What a texture is, for the texture packs: hashes of its data (with its format, size and
/// transparency flag) and of the palette it is read through (0 for direct colour).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) struct TexKey {
    pub data: u64,
    pub pal: u64,
    pub fmt: u8,
    pub w: u16,
    pub h: u16,
}

/// An RGBA picture.
pub(super) struct Image {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

/// A texel of a replacement image where the DS's texture coordinates (s, t in texels of the
/// original) point, wrapped as the texture's flags say: (r, g, b) in 6 bits, alpha in 5.
fn sample_image(img: &Image, tex: u32, s: f32, t: f32) -> (u32, u32, u32, u32) {
    let (ow, oh) = ((8u32 << ((tex >> 20) & 7)) as f32, (8u32 << ((tex >> 23) & 7)) as f32);
    let wrap = |v: i32, size: i32, repeat: bool, flip: bool| -> usize {
        let r = if repeat {
            let m = v.rem_euclid(size);
            if flip && v.div_euclid(size) & 1 != 0 { size - 1 - m } else { m }
        } else {
            v.clamp(0, size - 1)
        };
        r as usize
    };
    let x = wrap((s / ow * img.w as f32).floor() as i32, img.w as i32, tex & 0x10000 != 0, tex & 0x40000 != 0);
    let y = wrap((t / oh * img.h as f32).floor() as i32, img.h as i32, tex & 0x20000 != 0, tex & 0x80000 != 0);
    let p = &img.rgba[(y * img.w + x) * 4..(y * img.w + x) * 4 + 4];
    ((p[0] >> 2) as u32, (p[1] >> 2) as u32, (p[2] >> 2) as u32, (p[3] as u32 * 31 + 127) / 255)
}

/// What `render` leaves out, by bit of the mask it returns.
pub(super) const NOTE: [&str; 2] = [
    "",
    "3D: anti-aliasing is not drawn (at 2x and more the picture is smoother anyway)",
];

fn c5to6(c: u32) -> u32 {
    if c != 0 { c * 2 + 1 } else { 0 }
}

/// A 15-bit palette colour as RGB666.
fn rgb555(c: u16) -> (u32, u32, u32) {
    (c5to6(c as u32 & 31), c5to6(c as u32 >> 5 & 31), c5to6(c as u32 >> 10 & 31))
}

/// The texture memory a texel is read from: the texture slots and the texture palettes.
#[derive(Clone, Copy)]
pub(super) struct Mem<'a> {
    pub tex: &'a [u8],
    pub pal: &'a [u8],
}

fn pal_color(m: &Mem, off: usize) -> u16 {
    if off + 1 < m.pal.len() { u16::from_le_bytes([m.pal[off], m.pal[off + 1]]) } else { 0 }
}

fn tex8(m: &Mem, off: usize) -> u8 {
    m.tex.get(off & 0x7FFFF).copied().unwrap_or(0)
}

/// One texel: (r, g, b) in 6 bits and alpha in 5.
pub(super) fn sample(job: &Mem, tex: u32, pal: u32, s: f32, t: f32) -> (u32, u32, u32, u32) {
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
    /// Shadow polygons: where a mask polygon lay behind something.
    stencil: Vec<u8>,
    /// The pixel takes the fog (its polygon's attribute, or the clear plane's).
    fog: Vec<bool>,
    /// The pixel's depth is a translucent polygon's (the outline leaves such pixels alone).
    trans: Vec<bool>,
    /// The ID of the last opaque polygon at the pixel (a translucent one overwrites `id`).
    oid: Vec<u8>,
    /// How much of the scene the translucent polygons drawn over its last opaque pixel hide, 0-31.
    cover: Vec<u8>,
}

fn edge(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> f32 {
    (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
}

/// The rear plane's bitmap (DISP3DCNT bit 14): colours from texture slot 2, depths and fog flags
/// from slot 3, 256 x 256 starting at CLRIMAGE_OFFSET and wrapping; it replaces the clear colour
/// and depth where the picture has nothing nearer.
fn rear_plane(job: &Job, t: &mut Target, s: usize, ya: usize, yb: usize) {
    let (w, ox, oy) = (job.width * s, (job.clrimage_offset & 0xFF) as usize, (job.clrimage_offset >> 8) as usize);
    let cid = ((job.clear_color >> 24) & 63) as u8;
    let rd = |base: usize, x: usize, y: usize| -> u16 {
        let o = base + (y * 256 + x) * 2;
        u16::from_le_bytes([job.tex.get(o).copied().unwrap_or(0), job.tex.get(o + 1).copied().unwrap_or(0)])
    };
    for y in ya..yb {
        for x in 0..w {
            let (sx, sy) = ((x / s + ox) & 255, (y / s + oy) & 255);
            let (c, d) = (rd(0x40000, sx, sy), rd(0x60000, sx, sy));
            let (r, g, b) = rgb555(c & 0x7FFF);
            let i = (y - ya) * w + x;
            t.color[i] = r | g << 6 | b << 12;
            t.alpha[i] = if c & 0x8000 != 0 { 31 } else { 0 };
            t.depth[i] = clear_depth(d & 0x7FFF);
            t.id[i] = cid;
            t.fog[i] = d & 0x8000 != 0;
        }
    }
}

/// The fog density (0-128) at a depth: FOG_TABLE's 32 entries sit at FOG_OFFSET + (0x400 >> shift)
/// times 1 to 32, linear between; before the first and past the last, those entries.
fn fog_density(job: &Job, depth24: u32) -> u32 {
    let z = (depth24 >> 9) as i32;
    let step = (0x400i32 >> ((job.disp3dcnt >> 8) & 15)).max(1);
    let pos = z - job.fog_offset as i32 - step;
    let d = if pos <= 0 {
        job.fog_table[0] as i32 * 256
    } else {
        let (i, f) = ((pos / step) as usize, pos % step);
        if i >= 31 {
            job.fog_table[31] as i32 * 256
        } else {
            let (a, b) = (job.fog_table[i] as i32, job.fog_table[i + 1] as i32);
            a * 256 + (b - a) * 256 * f / step
        }
    };
    let d = (d / 256) as u32 & 127;
    if d >= 127 { 128 } else { d }
}

/// Fog over the finished picture: pixels that take it move towards the fog colour (unless the mode
/// says alpha only) and the fog alpha by the density at their depth.
fn apply_fog(job: &Job, t: &mut Target) {
    let (fr, fg, fb) = rgb555((job.fog_color & 0x7FFF) as u16);
    let fa = (job.fog_color >> 16) & 31;
    let alpha_only = job.disp3dcnt & 0x40 != 0;
    for i in 0..t.color.len() {
        if !t.fog[i] {
            continue;
        }
        let d = fog_density(job, (t.depth[i].clamp(0.0, 1.0) * 16_777_215.0) as u32);
        let mix = |a: u32, b: u32| (a * (128 - d) + b * d) >> 7;
        if !alpha_only {
            let c = t.color[i];
            t.color[i] = mix(c & 63, fr) | mix(c >> 6 & 63, fg) << 6 | mix(c >> 12 & 63, fb) << 12;
        }
        t.alpha[i] = mix(t.alpha[i] as u32, fa) as u8;
    }
}

/// How many threads draw the picture: the cores (at most 8); `SUNPETAL_3D_THREADS` sets it (1 draws
/// in one pass, to compare).
fn threads() -> usize {
    std::env::var("SUNPETAL_3D_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |p| p.get()))
        .clamp(1, 8)
}

/// The polygons drawn on the rows `ya..yb` of the picture (`s` times the DS's size).
fn raster(job: &Job, verts: &[GxVertex], s: usize, ya: usize, yb: usize) -> Target {
    let cc = job.clear_color;
    let (cr, cg, cb) = rgb555((cc & 0x7FFF) as u16);
    let calpha = ((cc >> 16) & 31) as u8;
    let cid = ((cc >> 24) & 63) as u8;
    let (w, h) = (job.width * s, 192 * s);
    let n = w * (yb - ya);
    let mut t = Target {
        color: vec![cr | cg << 6 | cb << 12; n],
        alpha: vec![calpha; n],
        depth: vec![clear_depth(job.clear_depth); n],
        id: vec![cid; n],
        stencil: vec![0; n],
        fog: vec![cc & 0x8000 != 0; n],
        trans: vec![false; n],
        oid: vec![cid; n],
        cover: vec![0; n],
    };
    if job.disp3dcnt & 0x4000 != 0 {
        rear_plane(job, &mut t, s, ya, yb);
    }
    let mem = Mem { tex: &job.tex, pal: &job.texpal };
    let texture_on = job.disp3dcnt & 1 != 0;
    let highlight = job.disp3dcnt & 2 != 0;
    let alpha_test = job.disp3dcnt & 4 != 0;
    let blend_on = job.disp3dcnt & 8 != 0;
    for p in &job.polys {
        let vs = &verts[p.first as usize..(p.first + p.n) as usize];
        let attr = p.attr;
        let polyalpha = ((attr >> 16) & 31) as u32;
        let pid = ((attr >> 24) & 63) as u8;
        let mode = (attr >> 4) & 3;
        let fmt = (p.tex >> 26) & 7;
        let textured = texture_on && fmt != 0;
        // a texture the pack replaces: sampled from its image
        let replacement: Option<Arc<Image>> = job.keys.get(&(p.tex, p.pal)).and_then(|k| job.pack.get(k));
        let eq_depth = attr & 0x4000 != 0;
        let fog_attr = attr & 0x8000 != 0;
        let write_trans_depth = attr & 0x800 != 0;
        let wire = polyalpha == 0;
        // fill one pixel
        let put = |t: &mut Target, x: usize, y: usize, depth: f32, vr: f32, vg: f32, vb: f32, s: f32, tt: f32| {
            let i = (y - ya) * w + x;
            let d = t.depth[i];
            let tol = 0x200 as f32 / 16_777_215.0;
            let behind = if eq_depth { (depth - d).abs() > tol } else { depth >= d };
            if mode == 3 && pid == 0 {
                // a shadow mask: no picture, a mark where it lies behind something
                if behind {
                    t.stencil[i] = 1;
                }
                return;
            }
            if behind {
                return;
            }
            if mode == 3 {
                // a shadow: only where a mask lay behind, and not on the polygon's own ID
                if t.stencil[i] == 0 || t.id[i] == pid {
                    return;
                }
            }
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
                let (tr, tg, tb, ta) = match replacement {
                    Some(ref img) => sample_image(img, p.tex, s, tt),
                    None => sample(&mem, p.tex, p.pal, s, tt),
                };
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
            if oa >= 31 {
                t.color[i] = or | og << 6 | ob << 12;
                t.alpha[i] = 31;
                t.depth[i] = depth;
                t.id[i] = pid;
                t.stencil[i] = 0;
                t.fog[i] = fog_attr;
                t.trans[i] = false;
                t.oid[i] = pid;
                t.cover[i] = 0;
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
                    t.cover[i] = t.cover[i].max(oa as u8);
                }
                if write_trans_depth {
                    t.depth[i] = depth;
                    t.trans[i] = true;
                }
                t.id[i] = pid;
                t.stencil[i] = 0;
                t.fog[i] = fog_attr;
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
                    if !(0..w as i32).contains(&x) || !(ya as i32..yb as i32).contains(&y) {
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
            let x1 = (pa.0.max(pb.0).max(pc.0).ceil().min(w as f32 - 1.0)) as usize;
            let y0 = (pa.1.min(pb.1).min(pc.1).floor().max(0.0) as usize).max(ya);
            let y1 = (pa.1.max(pb.1).max(pc.1).ceil().min(h as f32 - 1.0)) as usize;
            if y0 > y1.min(yb - 1) {
                continue;
            }
            let y1 = y1.min(yb - 1);
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
    t
}

/// How much nearer than the mean of its two neighbours a pixel's depth must be (0-1 of the 24
/// bits) to count as an object's edge.
const OUTLINE_STEP: f32 = 0.002;

/// Depth (0-1 of the 24 bits) under which a pixel is the game's interface drawn in 3D (flat quads
/// at the near plane: the logo at 0.0006, the panels at 0.01), not a part of the scene (which lies at
/// 0.9 and farther): no outline there.
const INTERFACE_DEPTH: f32 = 0.05;

/// The alpha (0-31) from which a translucent polygon over the scene hides it, and its outline, for good.
const HIDING_ALPHA: u8 = 12;

/// How strong the line along the 3D's border with the 2D layers is, against one between objects.
const SILHOUETTE: f32 = 0.5;

/// An outline of our own, beyond the DS's edge marking: each pixel's line level, 0-15 (0 none; the
/// compositor darkens the 3D's colour by it, only where the 3D is on show, in bits 24-27 of the
/// picture's word). A pixel on the near side of a jump in depth, or of another polygon ID, gets
/// `strength`; one beside a pixel with no 3D (the silhouette against the 2D layers) half of that. A plane's depth is a straight line over the
/// screen, so its second difference is zero and only the jumps show. Pixels whose depth a
/// translucent polygon wrote (the game's overlays and effects), and their neighbours, are not lines;
/// a translucent polygon over the picture does not change which polygon the pixel belongs to.
/// The game's interface drawn in 3D (`INTERFACE_DEPTH`) gets no line either.
/// The line is `width` DS pixels wide (one pixel at least).
fn outline_pass(strength: f32, width: f32, t: &Target, w: usize, h: usize, s: usize) -> Vec<u8> {
    let k = ((width * s as f32).round() as usize).max(1);
    let (depth, alpha, id, trans) = (t.depth.clone(), t.alpha.clone(), t.oid.clone(), t.trans.clone());
    // a translucent polygon over the pixel (the dialogue's plate, the glass) hides the scene, and the
    // line: weaker by its alpha, none from `HIDING_ALPHA`
    let level = |f: f32, cover: u8| {
        if cover >= HIDING_ALPHA {
            return 0;
        }
        ((strength * f).min(1.0) * (31 - cover) as f32 / 31.0 * 15.0).round() as u8
    };
    let mut lines = vec![0u8; w * h];
    for y in k..h.saturating_sub(k) {
        for x in k..w.saturating_sub(k) {
            let i = y * w + x;
            if alpha[i] < 31 || trans[i] || depth[i] < INTERFACE_DEPTH {
                continue;
            }
            let pairs = [(i - k, i + k), (i - k * w, i + k * w)];
            let (mut edge, mut silhouette) = (false, false);
            for (a, b) in pairs {
                if trans[a] || trans[b] || depth[a] < INTERFACE_DEPTH || depth[b] < INTERFACE_DEPTH {
                    continue;
                }
                if alpha[a] == 0 || alpha[b] == 0 {
                    silhouette = true;
                } else if depth[a] + depth[b] - 2.0 * depth[i] > OUTLINE_STEP * 2.0
                    || (id[a] != id[i] && depth[a] > depth[i])
                    || (id[b] != id[i] && depth[b] > depth[i])
                {
                    edge = true;
                }
            }
            if edge || silhouette {
                lines[i] = level(if edge { 1.0 } else { SILHOUETTE }, t.cover[i]);
            }
        }
    }
    lines
}

/// Soft shadows from the depth: each pixel's shade level, 0-15 (0 none; the compositor darkens the
/// 3D's colour by it, in bits 28-31 of the picture's word). The light is up and a little left, so
/// a shadow falls down and a little right: from a pixel three rays go back towards the light, up to
/// `radius` DS pixels, and one that meets something nearer than the pixel (by `JUMP` of depth)
/// shades it, the more the nearer to that something (a penumbra, the three rays apart). Only what
/// lies behind an object is shaded, never the object itself. Interface, translucent-depth and
/// empty pixels are not occluders. A translucent polygon over the pixel takes the shade as it
/// takes the outline.
fn shade_pass(strength: f32, radius: f32, t: &Target, w: usize, h: usize, s: usize) -> Vec<u8> {
    // how much nearer than the pixel something must be to cast on it: a slope's own change over the
    // ray is less
    const JUMP: f32 = 0.006;
    let steps = ((radius * s as f32).round() as usize).max(2);
    // towards the light: up and a little left, the rays a little apart
    let rays: Vec<(f32, f32)> = [-0.3f32, 0.0, 0.3]
        .iter()
        .map(|a| {
            let (c, sn) = (a.cos(), a.sin());
            let (dx, dy) = (-0.4f32, -0.92f32);
            (dx * c - dy * sn, dx * sn + dy * c)
        })
        .collect();
    let (wi, hi) = (w as isize, h as isize);
    let mut out = vec![0u8; w * h];
    let bands = threads().min(h / 64).max(1);
    let per = (h + bands - 1) / bands;
    let rays = &rays;
    std::thread::scope(|sc| {
        for (k, chunk) in out.chunks_mut(per * w).enumerate() {
            sc.spawn(move || {
                for (ry, row) in chunk.chunks_mut(w).enumerate() {
                    let y = (k * per + ry) as isize;
                    for x in 0..wi {
                        let i = (y * wi + x) as usize;
                        if t.alpha[i] < 31 || t.trans[i] || t.depth[i] < INTERFACE_DEPTH || t.cover[i] >= HIDING_ALPHA {
                            continue;
                        }
                        let d = t.depth[i];
                        let mut total = 0.0;
                        for &(dx, dy) in rays {
                            // the nearest occluder along the ray: its distance fades the shade
                            for n in 1..=steps {
                                let (qx, qy) = (x + (dx * n as f32).round() as isize, y + (dy * n as f32).round() as isize);
                                if qx < 0 || qy < 0 || qx >= wi || qy >= hi {
                                    break;
                                }
                                let j = (qy * wi + qx) as usize;
                                if t.alpha[j] < 31 || t.trans[j] || t.depth[j] < INTERFACE_DEPTH {
                                    continue;
                                }
                                if t.depth[j] < d - JUMP {
                                    total += 1.0 - (n - 1) as f32 / steps as f32;
                                    break;
                                }
                            }
                        }
                        let shade = total / rays.len() as f32;
                        let dim = (31 - t.cover[i]) as f32 / 31.0;
                        row[x as usize] = (shade * strength.min(1.0) * dim * 15.0).round() as u8;
                    }
                }
            });
        }
    });
    out
}

/// Draws a frame. Returns the picture and the mask of what it left out (`NOTE`).
pub(super) fn render(job: &Job, scale: usize) -> (Vec<u32>, u32) {
    let mut notes = 0u32;
    let s = scale.max(1);
    let (w, h) = (job.width * s, 192 * s);
    let n = w * h;
    let sc = s as f32;
    let verts: Vec<GxVertex> = job.verts.iter().map(|v| GxVertex { sx: v.sx * sc, sy: v.sy * sc, ..*v }).collect();
    if job.disp3dcnt & 0x10 != 0 {
        notes |= 1 << 1;
    }
    // horizontal bands, one thread each: every band draws all the polygons in order but only its own
    // rows, so the picture is the same as from one pass
    let bands = if s < 2 { 1 } else { threads().min(h / 64) };
    let per = (h + bands - 1) / bands;
    let ranges: Vec<(usize, usize)> = (0..bands).map(|k| (k * per, ((k + 1) * per).min(h))).filter(|r| r.0 < r.1).collect();
    let parts: Vec<Target> = if ranges.len() == 1 {
        vec![raster(job, &verts, s, 0, h)]
    } else {
        std::thread::scope(|sc| {
            let hs: Vec<_> = ranges.iter().map(|&(ya, yb)| { let v = &verts; sc.spawn(move || raster(job, v, s, ya, yb)) }).collect();
            hs.into_iter().map(|h| h.join().expect("a band of the 3D picture")).collect()
        })
    };
    let mut t = Target { color: Vec::with_capacity(n), alpha: Vec::with_capacity(n), depth: Vec::with_capacity(n), id: Vec::with_capacity(n), stencil: Vec::new(), fog: Vec::with_capacity(n), trans: Vec::with_capacity(n), oid: Vec::with_capacity(n), cover: Vec::with_capacity(n) };
    for p in parts {
        t.color.extend(p.color);
        t.alpha.extend(p.alpha);
        t.depth.extend(p.depth);
        t.id.extend(p.id);
        t.fog.extend(p.fog);
        t.trans.extend(p.trans);
        t.oid.extend(p.oid);
        t.cover.extend(p.cover);
    }
    if job.disp3dcnt & 0x20 != 0 {
        // edge marking: a pixel with a nearer-or-equal... no: a pixel beside a farther pixel of another
        // polygon ID (the clear plane's included) takes its ID's edge colour
        let (id, depth, alpha) = (t.id.clone(), t.depth.clone(), t.alpha.clone());
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if alpha[i] < 31 {
                    continue;
                }
                let nb = [
                    (x >= s).then(|| i - s),
                    (x + s < w).then(|| i + s),
                    (y >= s).then(|| i - s * w),
                    (y + s < h).then(|| i + s * w),
                ];
                if nb.iter().flatten().any(|&j| id[j] != id[i] && depth[j] >= depth[i]) {
                    let (r, g, b) = rgb555(job.edge[(id[i] >> 3) as usize & 7]);
                    t.color[i] = r | g << 6 | b << 12;
                }
            }
        }
    }
    let ex = job.extras;
    let lines = if ex.outline > 0.0 { outline_pass(ex.outline, ex.outline_width, &t, w, h, s) } else { Vec::new() };
    let shades = if ex.shadow > 0.0 { shade_pass(ex.shadow, ex.shadow_radius, &t, w, h, s) } else { Vec::new() };
    if job.disp3dcnt & 0x80 != 0 {
        apply_fog(job, &mut t);
    }
    let extra = |i: usize| (lines.get(i).map_or(0, |&l| l as u32) << 24) | (shades.get(i).map_or(0, |&l| l as u32) << 28);
    let out = (0..n).map(|i| if t.alpha[i] == 0 { 0 } else { t.color[i] | (t.alpha[i] as u32) << 18 | extra(i) }).collect();
    (out, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            extras: crate::nds::Extras::default(),
            edge: [0; 8],
            fog_color: 31 << 16,
            fog_offset: 0,
            fog_table: [0; 32],
            toon: [0; 32],
            tex: vec![0; 0x80000],
            texpal: vec![0; 0x18000],
            keys: HashMap::new(),
            pack: Arc::new(crate::nds::texpack::Pack::empty()),
        }
    }

    /// A job like `j` (the fields the fog reads).
    fn empty_job_with(j: &Job) -> Job {
        Job { fog_table: j.fog_table, disp3dcnt: j.disp3dcnt, width: j.width, ..empty_job() }
    }

    fn pixel(c555: u16, alpha: u32) -> u32 {
        let (r, g, b) = rgb555(c555);
        r | g << 6 | b << 12 | alpha << 18
    }

    const BLUE: u16 = 0x7C00;
    const RED: u16 = 0x001F;

    #[test]
    fn the_outline_marks_the_near_side_of_a_depth_jump_and_leaves_a_slope_alone() {
        let (w, h) = (8, 8);
        let mut t = Target { color: vec![0x3FFFF; w * h], alpha: vec![31; w * h], depth: vec![0.0; w * h], id: vec![0; w * h], stencil: vec![], fog: vec![false; w * h], trans: vec![false; w * h], oid: vec![0; w * h], cover: vec![0; w * h] };
        for y in 0..h {
            for x in 0..w {
                // a slope on the left, then a jump to something farther
                t.depth[y * w + x] = if x < 4 { 0.1 + 0.01 * x as f32 } else { 0.8 };
            }
        }
        let lines = outline_pass(1.0, 0.5, &t, w, h, 1);
        let dark = |x: usize| lines[3 * w + x] != 0;
        assert!(dark(3), "the near side of the jump");
        assert!(!dark(4), "the far side stays");
        assert!(!dark(1) && !dark(2), "a plane's slope is not an edge");
    }

    #[test]
    fn the_outline_leaves_the_interface_at_the_near_plane_alone() {
        let (w, h) = (8, 8);
        let mut t = Target { color: vec![0x3FFFF; w * h], alpha: vec![31; w * h], depth: vec![0.5; w * h], id: vec![0; w * h], stencil: vec![], fog: vec![false; w * h], trans: vec![false; w * h], oid: vec![0; w * h], cover: vec![0; w * h] };
        for y in 0..h {
            for x in 4..w {
                t.depth[y * w + x] = 0.0006;
            }
        }
        assert!(outline_pass(1.0, 0.5, &t, w, h, 1).iter().all(|&l| l == 0));
    }

    #[test]
    fn a_translucent_panel_over_the_scene_dims_the_line_under_it() {
        let (w, h) = (8, 8);
        let mut t = Target { color: vec![0x3FFFF; w * h], alpha: vec![31; w * h], depth: vec![0.0; w * h], id: vec![0; w * h], stencil: vec![], fog: vec![false; w * h], trans: vec![false; w * h], oid: vec![0; w * h], cover: vec![0; w * h] };
        for y in 0..h {
            for x in 0..w {
                t.depth[y * w + x] = if x < 4 { 0.9 } else { 0.99 };
            }
        }
        let bare = outline_pass(1.0, 0.5, &t, w, h, 1)[3 * w + 3];
        t.cover = vec![8; w * h];
        let under = outline_pass(1.0, 0.5, &t, w, h, 1)[3 * w + 3];
        assert!(bare == 15 && under > 0 && under < 15, "{bare} {under}");
        t.cover = vec![14; w * h];
        assert!(outline_pass(1.0, 0.5, &t, w, h, 1).iter().all(|&l| l == 0), "a plate that hides the scene hides its line");
    }

    #[test]
    fn a_shadow_falls_behind_an_object_away_from_the_light_and_never_on_it() {
        let (w, h) = (64, 64);
        let fresh = || Target { color: vec![0x3FFFF; w * h], alpha: vec![31; w * h], depth: vec![0.0; w * h], id: vec![0; w * h], stencil: vec![], fog: vec![false; w * h], trans: vec![false; w * h], oid: vec![0; w * h], cover: vec![0; w * h] };
        // a wall nearer by 0.05 over the top half, the floor under it: the shadow falls below the wall
        let mut t = fresh();
        for y in 0..h {
            for x in 0..w {
                t.depth[y * w + x] = if y < 24 { 0.85 } else { 0.9 };
            }
        }
        let shade = shade_pass(1.0, 6.0, &t, w, h, 1);
        let at = |x: usize, y: usize| shade[y * w + x];
        assert!(at(32, 25) > 0, "just under the wall's foot");
        assert!(at(32, 25) > at(32, 29), "fading away from it");
        assert_eq!(at(32, 40), 0, "far from it");
        assert_eq!(at(32, 10), 0, "the wall itself");
        // the same wall below the floor: the light comes from above, nothing falls on the floor over it
        let mut t = fresh();
        for y in 0..h {
            for x in 0..w {
                t.depth[y * w + x] = if y >= 40 { 0.85 } else { 0.9 };
            }
        }
        assert!(shade_pass(1.0, 6.0, &t, w, h, 1).iter().all(|&l| l == 0), "a shadow does not go towards the light");
        // a slope is not a shadow
        let mut t = fresh();
        for y in 0..h {
            for x in 0..w {
                t.depth[y * w + x] = 0.9 - 0.0005 * y as f32;
            }
        }
        assert!(shade_pass(1.0, 6.0, &t, w, h, 1).iter().all(|&l| l == 0));
    }

    #[test]
    fn the_outline_skips_translucent_pixels_and_draws_the_silhouette_lighter() {
        let (w, h) = (8, 8);
        let fresh = || Target { color: vec![0x3FFFF; w * h], alpha: vec![31; w * h], depth: vec![0.1; w * h], id: vec![0; w * h], stencil: vec![], fog: vec![false; w * h], trans: vec![false; w * h], oid: vec![0; w * h], cover: vec![0; w * h] };
        // a translucent strip of another ID over the picture: no line at its edge
        let mut t = fresh();
        for y in 0..h {
            for x in 4..w {
                t.oid[y * w + x] = 5;
                t.trans[y * w + x] = true;
            }
        }
        assert!(outline_pass(1.0, 0.5, &t, w, h, 1).iter().all(|&l| l == 0));
        // the border with the 2D layers: darker than nothing, lighter than an edge between objects
        let mut t = fresh();
        for y in 0..h {
            t.alpha[y * w + 4] = 0;
        }
        let l = outline_pass(1.0, 0.5, &t, w, h, 1)[3 * w + 3];
        assert!(l > 0 && l < 15, "lighter than a line between objects");    }

    #[test]
    fn fog_takes_the_clear_plane_to_the_fog_colour_when_the_plane_asks_for_it() {
        let mut j = empty_job();
        j.disp3dcnt = 0x80;
        j.clear_color = BLUE as u32 | 0x8000 | 31 << 16; // the clear plane's fog bit
        j.fog_color = RED as u32 | 31 << 16;
        j.fog_table = [127; 32];
        let (out, _) = render(&j, 1);
        assert_eq!(out[0], pixel(RED, 31), "full density: the fog colour");
        // without the plane's fog bit nothing changes
        j.clear_color = BLUE as u32 | 31 << 16;
        assert_eq!(render(&j, 1).0[0], pixel(BLUE, 31));
        // and fog switched off in DISP3DCNT does nothing either
        j.clear_color |= 0x8000;
        j.disp3dcnt = 0;
        assert_eq!(render(&j, 1).0[0], pixel(BLUE, 31));
    }

    #[test]
    fn half_density_is_half_way_and_alpha_only_leaves_the_colour() {
        let mut j = empty_job();
        j.disp3dcnt = 0x80;
        j.clear_color = BLUE as u32 | 0x8000 | 31 << 16;
        j.fog_color = RED as u32 | 31 << 16;
        j.fog_table = [64; 32];
        let (out, _) = render(&j, 1);
        let (br, _, bb) = (rgb555(BLUE).0, 0, rgb555(BLUE).2);
        let rr = rgb555(RED).0;
        assert_eq!(out[0] & 63, (br * 64 + rr * 64) >> 7);
        assert_eq!(out[0] >> 12 & 63, (bb * 64) >> 7);
        // alpha only, to a fog alpha of 0: the colour stays, the pixel goes transparent
        j.disp3dcnt = 0xC0;
        j.fog_color = RED as u32;
        j.fog_table = [127; 32];
        assert_eq!(render(&j, 1).0[0], 0);
        j.fog_table = [0; 32];
        assert_eq!(render(&j, 1).0[0], pixel(BLUE, 31));
    }

    #[test]
    fn the_density_follows_the_table_between_its_steps() {
        let mut j = empty_job();
        for i in 0..32 {
            j.fog_table[i] = (i * 4) as u8;
        }
        // shift 0: a step is 0x400; entry i sits at offset + 0x400 * (i + 1)
        let d = |z15: u32, j: &Job| fog_density(j, z15 << 9);
        assert_eq!(d(0, &j), 0, "before the first entry: the first");
        assert_eq!(d(0x800, &j), 4, "on entry 1");
        assert_eq!(d(0x800 + 0x200, &j), 6, "half way to entry 2");
        assert_eq!(d(0x7FFF, &j), 123, "the last entry sits at 0x8000, just past the deepest pixel");
        assert_eq!(d(0x7FFF, &Job { disp3dcnt: 1 << 8, ..empty_job_with(&j) }), 124, "with shift 1 the steps are half as long: past the last entry");
        // the offset moves the steps, the shift narrows them
        j.fog_offset = 0x100;
        assert_eq!(d(0x900, &j), 4);
        j.disp3dcnt = 1 << 8; // shift 1: a step is 0x200
        assert_eq!(d(0x100 + 0x400, &j), 4);
    }

    #[test]
    fn the_rear_plane_bitmap_replaces_the_clear_colour_and_scrolls() {
        let mut j = empty_job();
        j.disp3dcnt = 0x4000;
        j.clear_color = 31 << 16;
        // slot 2: pixel (1, 0) opaque red; slot 3: its depth 0 (nearest)
        let put = |tex: &mut Vec<u8>, base: usize, x: usize, v: u16| tex[base + 2 * x..base + 2 * x + 2].copy_from_slice(&v.to_le_bytes());
        put(&mut j.tex, 0x40000, 1, RED | 0x8000);
        put(&mut j.tex, 0x60000, 1, 0);
        let (out, _) = render(&j, 1);
        assert_eq!(out[0], 0, "the bitmap's first pixel has no alpha: nothing there");
        assert_eq!(out[1], pixel(RED, 31));
        // scrolled by one, the red pixel is at x = 0
        j.clrimage_offset = 1;
        assert_eq!(render(&j, 1).0[0], pixel(RED, 31));
        // drawn at scale 2 the pixel is two wide
        j.clrimage_offset = 0;
        let (big, _) = render(&j, 2);
        assert_eq!((big[2], big[3], big[1], big[4]), (pixel(RED, 31), pixel(RED, 31), 0, 0));
    }
}
