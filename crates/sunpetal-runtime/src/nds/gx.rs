//! The geometry engine, in software.
//!
//! Commands come packed through GXFIFO (0x04000400: a word of up to four command bytes, then
//! their parameters) or one at a time through the command ports (0x04000440-0x040005CC: each write
//! a parameter). They run at once, in fixed point as the DS computes: the matrix stacks
//! (projection, position and direction, texture), vertices in their five forms, texture
//! coordinates and their transforms, lighting at NORMAL, polygons from triangles, quads and their
//! strips, face culling, clipping against the view volume (the far plane by POLYGON_ATTR bit 12),
//! the screen position through the viewport, BOX_TEST, POS_TEST and VEC_TEST with their results.
//!
//! SWAP_BUFFERS waits for the VBlank: the commands that follow it wait in a queue (GXSTAT shows the
//! engine busy and the FIFO filling), then the polygon and vertex lists go to the rendering engine
//! and the queue runs. The rendering engine takes its lists, the 3D registers and the texture
//! memory at line 215 (`render3d`): what it draws is shown in the next frame.
//!
//! Not done: the 1-dot polygons' depth test, the lists' limits beyond the DS's own (2 048
//! polygons, 6 144 vertices: past them a polygon is dropped, as on the DS, and DISP3DCNT's
//! overflow bit set).
use std::collections::VecDeque;

use super::render3d::{GxPoly, GxVertex, Job};
use super::Nds;

const PARAMS: [u8; 128] = {
    let mut t = [0u8; 128];
    // 0x10: MTX_MODE PUSH POP STORE RESTORE IDENTITY LOAD44 LOAD43 MULT44 MULT43 MULT33 SCALE TRANS
    let a = [1u8, 0, 1, 1, 1, 0, 16, 12, 16, 12, 9, 3, 3];
    let mut i = 0;
    while i < a.len() {
        t[0x10 + i] = a[i];
        i += 1;
    }
    // 0x20: COLOR NORMAL TEXCOORD VTX16 VTX10 XY XZ YZ DIFF POLYGON_ATTR TEXIMAGE_PARAM PLTT_BASE
    let b = [1u8, 1, 1, 2, 1, 1, 1, 1, 1, 1, 1, 1];
    i = 0;
    while i < b.len() {
        t[0x20 + i] = b[i];
        i += 1;
    }
    // 0x30: DIF_AMB SPE_EMI LIGHT_VECTOR LIGHT_COLOR SHININESS
    let c = [1u8, 1, 1, 1, 32];
    i = 0;
    while i < c.len() {
        t[0x30 + i] = c[i];
        i += 1;
    }
    t[0x40] = 1; // BEGIN_VTXS
    t[0x50] = 1; // SWAP_BUFFERS
    t[0x60] = 1; // VIEWPORT
    t[0x70] = 3; // BOX_TEST
    t[0x71] = 2; // POS_TEST
    t[0x72] = 1; // VEC_TEST
    t
};

fn known(cmd: u32) -> bool {
    cmd == 0 || cmd == 0x11 || cmd == 0x15 || cmd == 0x41 || (cmd < 0x80 && PARAMS[cmd as usize] != 0)
}

type Mtx = [i32; 16];

const IDENTITY: Mtx = [0x1000, 0, 0, 0, 0, 0x1000, 0, 0, 0, 0, 0x1000, 0, 0, 0, 0, 0x1000];

/// m = n x m (n given, row-major, rows of four)
fn mult44(m: &mut Mtx, n: &Mtx) {
    let t = *m;
    for r in 0..4 {
        for c in 0..4 {
            m[r * 4 + c] =
                ((n[r * 4] as i64 * t[c] as i64 + n[r * 4 + 1] as i64 * t[4 + c] as i64 + n[r * 4 + 2] as i64 * t[8 + c] as i64 + n[r * 4 + 3] as i64 * t[12 + c] as i64) >> 12)
                    as i32;
        }
    }
}

fn mult43(m: &mut Mtx, n: &[i32]) {
    let mut f = [0i32; 16];
    for r in 0..4 {
        f[r * 4] = n[r * 3];
        f[r * 4 + 1] = n[r * 3 + 1];
        f[r * 4 + 2] = n[r * 3 + 2];
        f[r * 4 + 3] = if r == 3 { 0x1000 } else { 0 };
    }
    mult44(m, &f);
}

fn mult33(m: &mut Mtx, n: &[i32]) {
    let t = *m;
    for r in 0..3 {
        for c in 0..4 {
            m[r * 4 + c] = ((n[r * 3] as i64 * t[c] as i64 + n[r * 3 + 1] as i64 * t[4 + c] as i64 + n[r * 3 + 2] as i64 * t[8 + c] as i64) >> 12) as i32;
        }
    }
}

fn scale(m: &mut Mtx, s: &[i32]) {
    for r in 0..3 {
        for c in 0..4 {
            m[r * 4 + c] = ((m[r * 4 + c] as i64 * s[r] as i64) >> 12) as i32;
        }
    }
}

fn translate(m: &mut Mtx, v: &[i32]) {
    for c in 0..4 {
        m[12 + c] = ((m[c] as i64 * v[0] as i64 + m[4 + c] as i64 * v[1] as i64 + m[8 + c] as i64 * v[2] as i64 + ((m[12 + c] as i64) << 12)) >> 12) as i32;
    }
}

/// A vertex in clip coordinates with its attributes.
#[derive(Clone, Copy, Default)]
struct V {
    p: [i32; 4],
    c: [i32; 3],
    t: [i32; 2],
}

fn clip_point(inp: &V, out: &V, comp: usize, plane: i64) -> V {
    let num = inp.p[3] as i64 - plane * inp.p[comp] as i64;
    let den = num - (out.p[3] as i64 - plane * out.p[comp] as i64);
    let lerp = |a: i32, b: i32| -> i32 {
        if den != 0 {
            (a as i64 + ((b as i64 - a as i64) * num) / den) as i32
        } else {
            a
        }
    };
    let mut m = V::default();
    for i in 0..4 {
        m.p[i] = if i == comp { 0 } else { lerp(inp.p[i], out.p[i]) };
    }
    m.p[comp] = (plane * m.p[3] as i64) as i32;
    for i in 0..3 {
        m.c[i] = lerp(inp.c[i], out.c[i]);
    }
    for i in 0..2 {
        m.t[i] = lerp(inp.t[i], out.t[i]);
    }
    m
}

/// Sutherland-Hodgman against comp <= w (plane +1) then comp >= -w (plane -1).
fn clip_axis(v: &mut [V; 10], n: usize, comp: usize) -> usize {
    let mut tmp = [V::default(); 10];
    let mut m = 0;
    for i in 0..n {
        let (a, b) = (v[i], v[(i + 1) % n]);
        let (ain, bin) = (a.p[comp] <= a.p[3], b.p[comp] <= b.p[3]);
        if ain && m < 10 {
            tmp[m] = a;
            m += 1;
        }
        if ain != bin && m < 10 {
            tmp[m] = if ain { clip_point(&a, &b, comp, 1) } else { clip_point(&b, &a, comp, 1) };
            m += 1;
        }
    }
    let mut n = 0;
    for i in 0..m {
        let (a, b) = (tmp[i], tmp[(i + 1) % m]);
        let (ain, bin) = (a.p[comp] >= -a.p[3], b.p[comp] >= -b.p[3]);
        if ain && n < 10 {
            v[n] = a;
            n += 1;
        }
        if ain != bin && n < 10 {
            v[n] = if ain { clip_point(&a, &b, comp, -1) } else { clip_point(&b, &a, comp, -1) };
            n += 1;
        }
    }
    n
}

#[derive(Default)]
struct Lists {
    verts: Vec<GxVertex>,
    polys: Vec<GxPoly>,
    nverts_ds: u32,
    /// The batches' sources, in order: the address the batch's BEGIN_VTXS came from by DMA (0: not by DMA)
    /// the polygons it gave (`modelsrc.rs`) and the matrix-stack slot of the last MTX_RESTORE before it (the
    /// bone of a skinned part: 255 before any). A run of batches from one address and slot is one entry.
    srcs: Vec<(u32, u32, u8)>,
}

struct Queued {
    cmd: u32,
    p: Vec<u32>,
    /// `Gx::dl_src` as the command came.
    src: u32,
}

/// `SUNPETAL_GX_TRACE=FILE` (+ `SUNPETAL_GX_FRAMES=a-b`, by SWAP_BUFFERS' count): one text line
/// per BEGIN_VTXS..next batch, the vertices' box in model space (1.3.12 fixed point as sent),
/// the position matrix and the texture parameter. Reads the stream, changes nothing (K-003).
#[cfg(feature = "debug-tools")]
pub(super) struct Trace {
    out: std::io::BufWriter<std::fs::File>,
    lo: u64,
    hi: u64,
    open: Option<(i32, u32, [i16; 3], [i16; 3], Mtx, u32)>,
    /// `SUNPETAL_GX_VERTS=0xTEX`: also one `V` line per vertex of the batches with this texture parameter.
    vtex: Option<u32>,
    /// `SUNPETAL_GX_VERTS=all`: the vertices of every batch.
    vall: bool,
    batch: u32,
    /// The (TEXIMAGE_PARAM, PLTT_BASE) pairs of the traced frames' batches: at SWAP_BUFFERS each
    /// gets a `K` line with its texture key (the name of its picture in a texture pack).
    used: std::collections::HashSet<(u32, u32)>,
    /// With no `SUNPETAL_GX_FRAMES`: `FILE.go` appearing starts the trace for two frames (and is deleted).
    trigger: std::path::PathBuf,
}

#[cfg(feature = "debug-tools")]
impl Trace {
    fn from_env() -> Option<Trace> {
        let path = std::env::var_os("SUNPETAL_GX_TRACE")?;
        let (lo, hi) =
            std::env::var("SUNPETAL_GX_FRAMES").ok().and_then(|v| v.split_once('-').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))).unwrap_or((u64::MAX, u64::MAX));
        let trigger = std::path::PathBuf::from(format!("{}.go", path.to_string_lossy()));
        let verts = std::env::var("SUNPETAL_GX_VERTS").ok();
        let vall = verts.as_deref() == Some("all");
        let vtex = verts.and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok());
        Some(Trace { out: std::io::BufWriter::new(std::fs::File::create(path).ok()?), lo, hi, open: None, vtex, vall, batch: 0, used: Default::default(), trigger })
    }

    /// At SWAP_BUFFERS: the trigger file asks for the next two frames.
    fn poll(&mut self, frame: u64) {
        if self.lo == u64::MAX && self.trigger.exists() {
            let _ = std::fs::remove_file(&self.trigger);
            (self.lo, self.hi) = (frame + 1, frame + 2);
        }
    }

    fn flush(&mut self, frame: u64) {
        use std::io::Write;
        if let Some((prim, n, lo, hi, m, tex)) = self.open.take() {
            let _ = writeln!(self.out, "f={frame} vb={} b={} prim={prim} n={n} min={lo:?} max={hi:?} tex={tex:#010x} pos={m:?}", super::vblanks(), self.batch);
            self.batch += 1;
        }
    }

    fn vertex(&mut self, frame: u64, v: [i16; 3], m: &Mtx, tex: u32) {
        if frame < self.lo || frame > self.hi {
            return;
        }
        if let Some(o) = &mut self.open {
            o.1 += 1;
            for i in 0..3 {
                o.2[i] = o.2[i].min(v[i]);
                o.3[i] = o.3[i].max(v[i]);
            }
        }
        let _ = m;
        if self.vall || self.vtex == Some(tex) {
            use std::io::Write;
            let _ = writeln!(self.out, "V f={frame} b={} {} {} {}", self.batch, v[0], v[1], v[2]);
        }
    }
}

/// A PS2 model that stands for a DS one (`SPM2`; `docs/ULTIMATE-IMPORT.md`, steps У4.3 and У4.4), ready to
/// draw. `SUNPETAL_ULT_MODELS` names a folder of `*.spm` or a data file with a `MODL` part. The DS model's
/// batches are known by where the game sends them from (`modelsrc.rs`): they are built as ever (the counters
/// the game reads stay the same), taken out at the start of rendering, and this model's triangles go in
/// through the clip matrix of the anchor mesh. Its polygons do not count against the DS's list limits.
/// Not done: the model's pictures (it is drawn in its vertices' colours), its motions (the rest pose).
struct Ult {
    model: String,
    size: Option<u32>,
    anchor: Option<u16>,
    only_mesh: bool,
    /// What the model is made of, whatever it is bound to: two DS models may stand on one PS2 scene (a room and
    /// its furniture), and the scene is then drawn once (`ult_once`).
    source: u64,
    /// In the order of drawing (by pass).
    parts: Vec<UltPart>,
}

/// A material's triangles. Its picture is in the texture pack under `key`; the polygons carry `tex` and `pal`
/// (a TEXIMAGE_PARAM of the picture's size and a PLTT_BASE no DS texture has: `ULT_PAL`), which the frame's keys
/// lead from to the picture (`texture_keys`), so both rendering engines draw it as any replaced texture.
struct UltPart {
    key: Option<super::render3d::TexKey>,
    tex: u32,
    pal: u32,
    /// The polygon alpha (1-31): 31 but for a see-through material whose vertices say less.
    alpha: u32,
    /// Each vertex: its place in the anchor mesh's space, colour (0-31), texture coordinates (texels × 16).
    tris: Vec<[UltVertex; 3]>,
}

type UltVertex = ([f64; 3], [i32; 3], [i32; 2]);

/// The bit of a polygon's PLTT_BASE that says its picture is a replacement model's (the rest numbers the picture).
pub(super) const ULT_PAL: u32 = 0x8000_0000;
/// The format of a PS2 picture's key in a texture pack: `128x128_200_<picture hash>_<palette hash>.png`.
pub(super) const ULT_FMT: u8 = 200;

/// The bones' matrices in the model's space at rest (rows x, y, z of three, then the place): a bone is
/// T · Ry · Rz · Rx under its parent.
fn ult_bones(bones: &[sunpetal_data::spm2::Bone]) -> Vec<[f64; 12]> {
    fn local(r: &[f32; 6]) -> [f64; 12] {
        let (sx, cx) = (r[3] as f64).sin_cos();
        let (sy, cy) = (r[4] as f64).sin_cos();
        let (sz, cz) = (r[5] as f64).sin_cos();
        // Ry · Rz · Rx, row by row
        [
            cy * cz,
            -cy * sz * cx + sy * sx,
            cy * sz * sx + sy * cx,
            sz,
            cz * cx,
            -cz * sx,
            -sy * cz,
            sy * sz * cx + cy * sx,
            -sy * sz * sx + cy * cx,
            r[0] as f64,
            r[1] as f64,
            r[2] as f64,
        ]
    }
    fn mul(a: &[f64; 12], b: &[f64; 12]) -> [f64; 12] {
        let mut o = [0.0; 12];
        for r in 0..3 {
            for c in 0..3 {
                o[3 * r + c] = (0..3).map(|k| a[3 * r + k] * b[3 * k + c]).sum();
            }
            o[9 + r] = (0..3).map(|k| a[3 * r + k] * b[9 + k]).sum::<f64>() + a[9 + r];
        }
        o
    }
    fn world(bones: &[sunpetal_data::spm2::Bone], i: usize, done: &mut [Option<[f64; 12]>]) -> [f64; 12] {
        if let Some(m) = done[i] {
            return m;
        }
        let l = local(&bones[i].rest);
        let m = match bones[i].parent {
            Some(p) => mul(&world(bones, p as usize, done), &l),
            None => l,
        };
        done[i] = Some(m);
        m
    }
    let mut done = vec![None; bones.len()];
    (0..bones.len()).map(|i| world(bones, i, &mut done)).collect()
}

/// A point through a matrix of `ult_bones`' layout.
fn ult_point(m: &[f64; 12], v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|r| m[3 * r] * v[0] + m[3 * r + 1] * v[1] + m[3 * r + 2] * v[2] + m[9 + r])
}

impl Ult {
    /// `pictures` counts the materials with a picture, over all the models (their PLTT_BASE numbers).
    fn new(m: &sunpetal_data::spm2::Model, pictures: &mut u32) -> Option<Ult> {
        let b = m.bind.as_ref()?;
        let bones = ult_bones(&m.bones);
        // the binding's rows are the DS axes' (x, y, z of three), then the shift
        let place: [f64; 12] = std::array::from_fn(|i| b.place[i] as f64);
        let vert = |i: u32| {
            let v = &m.verts[i as usize];
            let mut p = [v.pos[0] as f64, v.pos[1] as f64, v.pos[2] as f64];
            if let Some(w) = bones.get(v.bone as usize) {
                p = ult_point(w, p);
            }
            let p = ult_point(&place, p.map(|x| x * m.unit as f64));
            (p, [0, 1, 2].map(|k| ((v.rgba[k] as i32 * 31 + 64) / 128).min(31)), v.uv)
        };
        // a material's picture: a TEXIMAGE_PARAM of a size that holds it (direct colour, repeated unless clamped)
        let code = |n: u16| (0..7u32).find(|c| 8u32 << c >= n as u32).unwrap_or(7);
        let mats: Vec<(Option<super::render3d::TexKey>, u32, u32)> = m
            .materials
            .iter()
            .map(|mt| match mt.picture {
                Some(pic) => {
                    *pictures += 1;
                    let repeat = (mt.flags & 2 == 0) as u32 | ((mt.flags & 4 == 0) as u32) << 1;
                    let tex = 7 << 26 | code(pic.h) << 23 | code(pic.w) << 20 | repeat << 16;
                    (Some(super::render3d::TexKey { data: pic.data, pal: pic.pal, fmt: ULT_FMT, w: pic.w, h: pic.h }), tex, ULT_PAL | *pictures)
                }
                None => (None, 0, 0),
            })
            .collect();
        let mut order: Vec<&sunpetal_data::spm2::Part> = m.parts.iter().collect();
        // the solid and cut-out materials first, the see-through ones over them (a pass at a time within each)
        order.sort_by_key(|p| {
            let mt = &m.materials[p.material as usize];
            (mt.blend == 2, mt.pass)
        });
        let mut parts = Vec::with_capacity(order.len());
        for p in order {
            let (key, tex, pal) = mats[p.material as usize];
            let (w, h) = ((8u32 << ((tex >> 20) & 7)) as f32 * 16.0, (8u32 << ((tex >> 23) & 7)) as f32 * 16.0);
            let texel = |(q, c, uv): ([f64; 3], [i32; 3], [f32; 2])| -> UltVertex {
                (q, c, [(uv[0] * w).round().clamp(-32768.0, 32767.0) as i32, (uv[1] * h).round().clamp(-32768.0, 32767.0) as i32])
            };
            let tris = m.idx[p.first as usize..(p.first + p.count) as usize].chunks_exact(3).map(|t| [texel(vert(t[0])), texel(vert(t[1])), texel(vert(t[2]))]).collect();
            // a see-through material's vertices carry its alpha (0x80 = 1.0 on the PS2); the picture's own
            // alpha comes through the texture
            let alpha = if m.materials[p.material as usize].blend == 2 {
                let ix = &m.idx[p.first as usize..(p.first + p.count) as usize];
                let sum: u64 = ix.iter().map(|&i| m.verts[i as usize].rgba[3] as u64).sum();
                ((sum * 31 / (ix.len().max(1) as u64 * 128)) as u32).clamp(1, 31)
            } else {
                31
            };
            parts.push(UltPart { key, tex, pal, alpha, tris });
        }
        // FNV-1a over the places and the triangles
        let mut source = 0xcbf2_9ce4_8422_2325u64;
        let bytes = m.verts.iter().flat_map(|v| v.pos).flat_map(f32::to_le_bytes).chain(m.idx.iter().flat_map(|i| i.to_le_bytes()));
        for x in bytes {
            source = (source ^ x as u64).wrapping_mul(0x0100_0000_01b3);
        }
        Some(Ult { model: b.model.clone(), size: b.size, anchor: b.mesh, only_mesh: b.only_mesh && b.mesh.is_some(), source, parts })
    }

    fn from_env() -> Vec<Ult> {
        let Some(path) = std::env::var_os("SUNPETAL_ULT_MODELS").map(std::path::PathBuf::from) else { return Vec::new() };
        let read = || -> Result<Vec<sunpetal_data::spm2::Model>, String> {
            let mut out = Vec::new();
            if path.is_dir() {
                let mut names: Vec<std::path::PathBuf> =
                    std::fs::read_dir(&path).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "spm")).collect();
                names.sort();
                for n in names {
                    let d = std::fs::read(&n).map_err(|e| format!("{}: {}", n.display(), e))?;
                    if d.starts_with(sunpetal_data::spm2::MAGIC) {
                        out.push(sunpetal_data::spm2::Model::decode(&n.file_stem().unwrap_or_default().to_string_lossy(), &d)?);
                    }
                }
            } else {
                let c = sunpetal_data::container::Container::read_only(&path, |t| t == sunpetal_data::models::TAG)?;
                for s in c.all(sunpetal_data::models::TAG) {
                    out.extend(sunpetal_data::spm2::unpack(&s.data)?);
                }
            }
            Ok(out)
        };
        match read() {
            Ok(models) => {
                let mut pictures = 0;
                let ult: Vec<Ult> = models.iter().filter_map(|m| Ult::new(m, &mut pictures)).collect();
                eprintln!("models: {} of {} from {} stand for DS models", ult.len(), models.len(), path.display());
                ult
            }
            Err(e) => {
                eprintln!("models: {}: {}", path.display(), e);
                Vec::new()
            }
        }
    }
}

/// Of the frame's replacements, the first of each source stays: the DS models that stand on one PS2 scene are
/// all hidden, and the scene is drawn once.
/// The "on air" scenes of the DS darken the whole 3D picture with a black see-through polygon over the screen,
/// and the DS rooms are lit to make up for it. A PS2 room carries its own light in the vertices, so under a
/// replacement the darkening is taken out (the picture is then the PS2's). It is known by what goes with it:
/// other see-through polygons without a picture (the bars and stripes). `SUNPETAL_ULT_DIM=keep` leaves it.
fn ult_undim(r: &mut Lists) {
    static KEEP: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *KEEP.get_or_init(|| std::env::var_os("SUNPETAL_ULT_DIM").is_some_and(|v| v == "keep")) {
        return;
    }
    let plain = |p: &GxPoly| (p.tex >> 26) & 7 == 0 && (1..31).contains(&((p.attr >> 16) & 31));
    let dim = |p: &GxPoly, v: &[GxVertex]| {
        let v = &v[p.first as usize..(p.first + p.n) as usize];
        let (x0, x1) = (v.iter().map(|g| g.ix).min().unwrap_or(0), v.iter().map(|g| g.ix).max().unwrap_or(0));
        let (y0, y1) = (v.iter().map(|g| g.iy).min().unwrap_or(0), v.iter().map(|g| g.iy).max().unwrap_or(0));
        plain(p)
            && (12..=20).contains(&((p.attr >> 16) & 31))
            && x1 as i32 - x0 as i32 >= 256
            && y1 as i32 - y0 as i32 >= 192
            && v.iter().all(|g| g.r < 1.0 && g.g < 1.0 && g.b < 1.0)
    };
    let verts = &r.verts;
    if r.polys.iter().any(|p| plain(p) && !dim(p, verts)) {
        r.polys.retain(|p| !dim(p, verts));
    }
}

fn ult_once<T>(at: &mut Vec<T>, source: impl Fn(&T) -> u64) {
    let mut seen: Vec<u64> = Vec::new();
    at.retain(|c| {
        let s = source(c);
        !seen.contains(&s) && {
            seen.push(s);
            true
        }
    });
}

/// A batch whose display list was copied to GXFIFO by the CPU: where its polygons begin, its entry in the frame's
/// sources, its clip matrix and polygon attributes, and the words that followed its BEGIN_VTXS (six at most).
struct UltPend {
    start: usize,
    src_at: usize,
    m: Mtx,
    attr: u32,
    words: Vec<u32>,
}

/// K-111, the prototype of a model swap at the GX border (`SUNPETAL_MODEL_SWAP=FILE`, off by
/// default): the batches that use the texture `tex` are not drawn; at the first of them in a frame
/// the file's triangles take their place, moved by `e` into the first batch's model space and
/// drawn through its matrix. The file: `tex HEX`, `size N` (the texture's side), `e` and twelve
/// numbers (rows x, y, z, then the shift), then `v x y z u v r g b` for every vertex, three to a
/// triangle (already in the DS's winding).
pub(super) struct ModelSwap {
    tex: u32,
    /// With `key NAME` (a texture pack's picture name, no `.png`): the batches whose texture has
    /// this key, wherever it lies in the texture memory (`tex` is then not looked at).
    key: Option<super::render3d::TexKey>,
    /// `hide NAME` lines: pictures whose batches are only taken out (a reflection slab, say).
    hide: Vec<super::render3d::TexKey>,
    /// The first matched batch of the frame: clip matrix, polygon attributes, texture, palette.
    at: Vec<(Mtx, u32, u32, u32, usize)>,
    size: f64,
    e: [f64; 12],
    verts: Vec<[f64; 8]>,
}

impl ModelSwap {
    fn from_env() -> Option<ModelSwap> {
        let text = std::fs::read_to_string(std::env::var_os("SUNPETAL_MODEL_SWAP")?).ok()?;
        let mut sw = ModelSwap { tex: 0, key: None, hide: Vec::new(), at: Vec::new(), size: 128.0, e: [0.0; 12], verts: Vec::new() };
        for line in text.lines() {
            let mut w = line.split_whitespace();
            match w.next() {
                Some("tex") => sw.tex = u32::from_str_radix(w.next()?, 16).ok()?,
                Some("key") => sw.key = super::texpack::parse_name(&format!("{}.png", w.next()?)),
                Some("hide") => sw.hide.extend(super::texpack::parse_name(&format!("{}.png", w.next()?))),
                Some("size") => sw.size = w.next()?.parse().ok()?,
                Some("e") => {
                    for (i, x) in w.take(12).enumerate() {
                        sw.e[i] = x.parse().ok()?;
                    }
                }
                Some("v") => {
                    let n: Vec<f64> = w.filter_map(|x| x.parse().ok()).collect();
                    if n.len() == 8 {
                        sw.verts.push([n[0], n[1], n[2], n[3], n[4], n[5], n[6], n[7]]);
                    }
                }
                _ => {}
            }
        }
        (sw.verts.len() >= 3).then_some(sw)
    }
}

pub(super) struct Gx {
    swap: Option<ModelSwap>,
    swap_skip: bool,
    swap_hide_r: Vec<(usize, usize)>,
    swap_at_r: Option<(Mtx, u32, u32, u32)>,
    swap_cur: Option<usize>,
    swap_open: Option<usize>,
    swap_hide: Vec<(usize, usize)>,
    swap_cache: std::collections::HashMap<(u32, u32), u8>,
    swap_gen: u64,
    swap_count: std::collections::HashMap<u32, u32>,
    swap_mtime: Option<std::time::SystemTime>,
    ult: Vec<Ult>,
    files: super::modelsrc::Files,
    /// The open run of a replaced model's polygons: where it began in the list, and its entry of `ult_seen`.
    ult_open: Option<(usize, usize)>,
    ult_hide: Vec<(usize, usize)>,
    /// The replaced batches of the frame being built: model, DS mesh, clip matrix, polygon attributes, polygons.
    ult_seen: Vec<(usize, u16, Mtx, u32, usize)>,
    ult_hide_r: Vec<(usize, usize)>,
    ult_at_r: Vec<(usize, Mtx, u32)>,
    /// A replacement's polygons are going in: the DS's list limits do not hold.
    unbounded: bool,
    /// A batch that did not come by DMA, to be known by its words when it ends (`ult_pend_close`).
    ult_pend: Option<UltPend>,
    #[cfg(feature = "debug-tools")]
    trace: Option<Trace>,
    /// 0x04000320-0x0400071F, as written.
    regs: Vec<u8>,
    mtx_proj: Mtx,
    mtx_pos: Mtx,
    mtx_vec: Mtx,
    mtx_tex: Mtx,
    mtx_clip: Mtx,
    stk_proj: Mtx,
    stk_tex: Mtx,
    stk_pos: [Mtx; 32],
    /// The slot of the last MTX_RESTORE (255: none yet), for `Build::srcs`.
    last_restore: u8,
    stk_vec: [Mtx; 32],
    sp_proj: u32,
    sp_tex: u32,
    sp_pos: u32,
    clip_dirty: bool,
    mtx_mode: u32,
    /// GXSTAT's own bits: the test result (1), the stack error (15), IRQ mode (30-31).
    stat: u32,
    // vertex state
    vtx: [i16; 3],
    raw_tc: [i16; 2],
    tc: [i16; 2],
    normal: [i16; 3],
    vcolor: [u8; 3],
    attr_pending: u32,
    attr: u32,
    tex_param: u32,
    pltt_base: u32,
    /// BEGIN_VTXS's type, -1 outside.
    prim: i32,
    /// x1, y1 (top), x2, y2 (bottom), as the screen counts.
    viewport: [u8; 4],
    /// Pixels the view is widened by each side (`nds::set_wide`).
    wide: usize,
    // lighting
    light_vec: [[i32; 3]; 4],
    light_col: [[u8; 3]; 4],
    diffuse: [u8; 3],
    ambient: [u8; 3],
    specular: [u8; 3],
    emission: [u8; 3],
    shine_table_on: bool,
    shine_table: [u8; 128],
    // the lists being built, and the rendering engine's
    build: Lists,
    render: Lists,
    render_wbuf: bool,
    render_manual: bool,
    swaps: u64,
    swap_pending: bool,
    /// The address of the word a DMA is writing to GXFIFO now (0: no DMA), and that of the command being run.
    pub(super) dl_src: u32,
    cmd_src: u32,
    swap_param: u32,
    // a strip's or quad's vertices so far
    strip: [V; 4],
    nstrip: usize,
    strip_count: u32,
    // the FIFO's decoder and the queue behind a swap
    cmds: u32,
    ncmds: i32,
    cur: i32,
    need: usize,
    got: usize,
    params: [u32; 32],
    queue: VecDeque<Queued>,
    port_got: Vec<usize>,
    port_params: Vec<[u32; 32]>,
    pos_result: [i32; 4],
    vec_result: [i16; 3],
}

impl Gx {
    pub(super) fn set_wide(&mut self, each_side: usize) {
        self.wide = each_side;
    }

    pub(super) fn new() -> Gx {
        Gx {
            swap: ModelSwap::from_env(),
            swap_skip: false,
            swap_hide_r: Vec::new(),
            swap_at_r: None,
            swap_cur: None,
            swap_open: None,
            swap_hide: Vec::new(),
            swap_cache: Default::default(),
            swap_gen: u64::MAX,
            swap_count: Default::default(),
            swap_mtime: None,
            ult: Ult::from_env(),
            files: Default::default(),
            ult_open: None,
            ult_hide: Vec::new(),
            ult_seen: Vec::new(),
            ult_hide_r: Vec::new(),
            ult_at_r: Vec::new(),
            unbounded: false,
            ult_pend: None,
            #[cfg(feature = "debug-tools")]
            trace: Trace::from_env(),
            regs: vec![0; 0x400],
            mtx_proj: IDENTITY,
            mtx_pos: IDENTITY,
            mtx_vec: IDENTITY,
            mtx_tex: IDENTITY,
            mtx_clip: IDENTITY,
            stk_proj: IDENTITY,
            stk_tex: IDENTITY,
            stk_pos: [IDENTITY; 32],
            last_restore: 255,
            stk_vec: [IDENTITY; 32],
            sp_proj: 0,
            sp_tex: 0,
            sp_pos: 0,
            clip_dirty: true,
            mtx_mode: 0,
            stat: 0,
            vtx: [0; 3],
            raw_tc: [0; 2],
            tc: [0; 2],
            normal: [0; 3],
            vcolor: [0; 3],
            attr_pending: 0,
            attr: 0,
            tex_param: 0,
            pltt_base: 0,
            prim: -1,
            viewport: [0, 0, 255, 191],
            wide: 0,
            light_vec: [[0; 3]; 4],
            light_col: [[0; 3]; 4],
            diffuse: [0; 3],
            ambient: [0; 3],
            specular: [0; 3],
            emission: [0; 3],
            shine_table_on: false,
            shine_table: [0; 128],
            build: Lists::default(),
            render: Lists::default(),
            render_wbuf: false,
            render_manual: false,
            swaps: 0,
            swap_pending: false,
            dl_src: 0,
            cmd_src: 0,
            swap_param: 0,
            strip: [V::default(); 4],
            nstrip: 0,
            strip_count: 0,
            cmds: 0,
            ncmds: 0,
            cur: -1,
            need: 0,
            got: 0,
            params: [0; 32],
            queue: VecDeque::new(),
            port_got: vec![0; 0x80],
            port_params: vec![[0; 32]; 0x80],
            pos_result: [0; 4],
            vec_result: [0; 3],
        }
    }

    /// Closes the open run of swapped polygons (and counts them for their matrix).
    fn swap_close(&mut self) {
        if let Some(s0) = self.swap_open.take() {
            let n = self.build.polys.len();
            if n > s0 {
                self.swap_hide.push((s0, n));
                if let (Some(sw), Some(i)) = (&mut self.swap, self.swap_cur) {
                    sw.at[i].4 += n - s0;
                }
            }
        }
        self.swap_cur = None;
    }

    /// The replacement bound to the model of file `fi`, if any (looked up once a file).
    fn ult_of(&self, files: &mut super::modelsrc::Files, fi: usize) -> Option<usize> {
        let f = &mut files.list[fi];
        *f.ult.get_or_insert_with(|| self.ult.iter().position(|u| u.model == f.model && u.size.is_none_or(|s| s == f.size)))
    }

    /// The entry of `ult_seen` for these batches (made if new).
    fn ult_seen_at(&mut self, u: usize, mesh: u16, m: Mtx, attr: u32) -> usize {
        match self.ult_seen.iter().position(|c| c.0 == u && c.1 == mesh && c.2 == m) {
            Some(i) => i,
            None => {
                self.ult_seen.push((u, mesh, m, attr, 0));
                self.ult_seen.len() - 1
            }
        }
    }

    /// Closes the open run of a replaced model's polygons.
    fn ult_close(&mut self) {
        if let Some((s0, i)) = self.ult_open.take() {
            let n = self.build.polys.len();
            if n > s0 {
                self.ult_hide.push((s0, n));
                self.ult_seen[i].4 += n - s0;
            }
        }
    }

    fn update_clip(&mut self) {
        if !self.clip_dirty {
            return;
        }
        self.mtx_clip = self.mtx_proj;
        let pos = self.mtx_pos;
        mult44(&mut self.mtx_clip, &pos);
        self.clip_dirty = false;
    }

    /// Applies `f` to the current matrix (or, in mode 2, both position and direction when
    /// `both_in_mode2`).
    fn on_current(&mut self, both_in_mode2: bool, f: impl Fn(&mut Mtx)) {
        match self.mtx_mode {
            0 => {
                f(&mut self.mtx_proj);
                self.clip_dirty = true;
            }
            1 => {
                f(&mut self.mtx_pos);
                self.clip_dirty = true;
            }
            2 => {
                f(&mut self.mtx_pos);
                if both_in_mode2 {
                    f(&mut self.mtx_vec);
                }
                self.clip_dirty = true;
            }
            _ => f(&mut self.mtx_tex),
        }
    }

    fn lighting(&mut self) {
        let m = self.mtx_vec;
        let nm = self.normal;
        let mut n = [0i32; 3];
        for i in 0..3 {
            n[i] = ((nm[0] as i64 * m[i] as i64 + nm[1] as i64 * m[4 + i] as i64 + nm[2] as i64 * m[8 + i] as i64) >> 12) as i32;
        }
        let mut c = [self.emission[0] as i32, self.emission[1] as i32, self.emission[2] as i32];
        for l in 0..4 {
            if self.attr & (1 << l) == 0 {
                continue;
            }
            let lv = self.light_vec[l];
            let mut dl = -(((lv[0] as i64 * n[0] as i64 + lv[1] as i64 * n[1] as i64 + lv[2] as i64 * n[2] as i64) >> 10) as i32);
            dl = dl.clamp(0, 255);
            let mut sl = -((((lv[0] >> 1) as i64 * n[0] as i64 + (lv[1] >> 1) as i64 * n[1] as i64 + ((lv[2] - 0x200) >> 1) as i64 * n[2] as i64) >> 10) as i32);
            if sl < 0 {
                sl = 0;
            } else if sl > 255 {
                sl = (0x100 - sl) & 0xFF;
            }
            sl = ((sl * sl) >> 7) - 0x100;
            if sl < 0 {
                sl = 0;
            }
            if self.shine_table_on {
                sl = self.shine_table[(sl >> 1) as usize] as i32;
            }
            for i in 0..3 {
                let lc = self.light_col[l][i] as i32;
                c[i] += (self.specular[i] as i32 * lc * sl) >> 13;
                c[i] += (self.diffuse[i] as i32 * lc * dl) >> 13;
                c[i] += (self.ambient[i] as i32 * lc) >> 5;
            }
        }
        for i in 0..3 {
            self.vcolor[i] = c[i].min(31) as u8;
        }
    }

    // ---- polygons ----------------------------------------------------------------------------
    /// Returns true if the polygon list overflowed (the caller sets DISP3DCNT's bit).
    fn submit_poly(&mut self, inp: &[V], n: usize) -> bool {
        // face culling, on the clip coordinates (melonDS's form: the normal of the triangle
        // (x, y, w) of the first three vertices, against the eye)
        let (v0, v1, v2) = (&inp[0], &inp[1], &inp[2]);
        let p = |a: i32, b: i32| (a - b) as i64;
        let mut nx = p(v0.p[1], v1.p[1]) * p(v2.p[3], v1.p[3]) - p(v0.p[3], v1.p[3]) * p(v2.p[1], v1.p[1]);
        let mut ny = p(v0.p[3], v1.p[3]) * p(v2.p[0], v1.p[0]) - p(v0.p[0], v1.p[0]) * p(v2.p[3], v1.p[3]);
        let mut nz = p(v0.p[0], v1.p[0]) * p(v2.p[1], v1.p[1]) - p(v0.p[1], v1.p[1]) * p(v2.p[0], v1.p[0]);
        let big = |x: i64| (x >> 31) ^ (x >> 63) != 0;
        while big(nx) || big(ny) || big(nz) {
            nx >>= 4;
            ny >>= 4;
            nz >>= 4;
        }
        let dot = v1.p[0] as i64 * nx + v1.p[1] as i64 * ny + v1.p[3] as i64 * nz;
        let front = dot < 0;
        let attr = self.attr;
        let drop = if front {
            attr & 0x80 == 0
        } else if dot > 0 {
            attr & 0x40 == 0
        } else {
            attr & 0xC0 == 0
        };
        if drop {
            return false;
        }
        let mut v = [V::default(); 10];
        v[..n].copy_from_slice(&inp[..n]);
        // a widened view: x is taken in a frustum that much wider, so the clipping lets the sides in
        let big_w = 256 + 2 * self.wide as i64;
        if self.wide > 0 {
            // the game's own panels and rules (flat, w the same all over, running to the screen's
            // edge) run to the edge of the wider view too; everything else keeps its place
            let flat = v[..n].iter().all(|x| x.p[3] == v[0].p[3]);
            let (left, right) = (v[..n].iter().any(|x| x.p[0] == -x.p[3]), v[..n].iter().any(|x| x.p[0] == x.p[3]));
            // only the untextured ones (a picture would be stretched), and a translucent one only when
            // it is a band across most of the screen (a panel), not a narrow strip (it would veil the sides)
            let untextured = (self.tex_param >> 26) & 7 == 0;
            let wide_enough = v[..n].iter().map(|x| x.p[0]).max() > v[..n].iter().map(|x| x.p[0] + v[0].p[3]).min();
            // ... or a strip that is not tall (the glitch bands of the title's FSR screen start at the edge, however short)
            let tall = {
                let (lo, hi) = (v[..n].iter().map(|x| x.p[1]).min().unwrap_or(0), v[..n].iter().map(|x| x.p[1]).max().unwrap_or(0));
                (hi as i64 - lo as i64) * 100 >= v[0].p[3] as i64 * 2 * 90
            };
            let stretch = flat && untextured && (left || right) && ((attr >> 16) & 31 == 31 || wide_enough || !tall);
            // a band across most of the screen goes to both edges, even if it starts a little inside
            // (the dialogue panel starts beside the portrait)
            let (min_x, max_x) = (v[..n].iter().map(|x| x.p[0]).min().unwrap_or(0), v[..n].iter().map(|x| x.p[0]).max().unwrap_or(0));
            let w0 = v[0].p[3];
            let (on_left, on_right) = (max_x as i64 * 10 <= -(w0 as i64) * 2, min_x as i64 * 10 >= w0 as i64 * 2);
            let alpha_here = (attr >> 16) & 31;
            let edge_strip = flat && untextured && !stretch && tall && (on_left != on_right) && alpha_here > 0 && alpha_here < 31;
            for x in &mut v[..n] {
                if stretch {
                    if x.p[0].abs() == x.p[3] {
                        continue;
                    }
                    if wide_enough && (x.p[0] == min_x || x.p[0] == max_x) {
                        x.p[0] = if x.p[0] == min_x { -x.p[3] } else { x.p[3] };
                        continue;
                    }
                }
                x.p[0] = (x.p[0] as i64 * 256 / big_w) as i32;
                // a strip the game lays against one edge of its picture (the flickering bars of the "on air"
                // frame, the wipe of the dialogue panel) stays against that edge of the wider view
                if edge_strip {
                    let w = x.p[3] as i64;
                    x.p[0] -= (w - w * 256 / big_w) as i32 * if on_left { 1 } else { -1 };
                }
            }
        }
        if attr & 0x1000 == 0 && v[..n].iter().any(|x| x.p[2] > x.p[3]) {
            return false; // past the far plane, and not to be drawn cut
        }
        let mut n = clip_axis(&mut v, n, 2);
        if n != 0 {
            n = clip_axis(&mut v, n, 0);
        }
        if n != 0 {
            n = clip_axis(&mut v, n, 1);
        }
        if n < 3 {
            return false;
        }
        if !self.unbounded && (self.build.polys.len() >= 2048 || self.build.nverts_ds + n as u32 > 6144) {
            return true;
        }
        let fmt = (self.tex_param >> 26) & 7;
        let alpha = (attr >> 16) & 31;
        let mut poly = GxPoly {
            first: self.build.verts.len() as u32,
            n: n as u32,
            attr,
            tex: self.tex_param,
            pal: self.pltt_base,
            translucent: (alpha > 0 && alpha < 31) || fmt == 1 || fmt == 6,
            sortkey: 0,
        };
        let (vx, vy) = (self.viewport[0] as i64, self.viewport[1] as i64);
        let vw0 = (self.viewport[2] as i64 - self.viewport[0] as i64 + 1) & 0x1FF;
        // widened: the viewport as wide again as the frustum; the picture keeps its place, with the
        // new columns each side of it
        let vw = vw0 * big_w / 256;
        let vx = vx + self.wide as i64 - (vw - vw0) / 2;
        let vh = (self.viewport[3] as i64 - self.viewport[1] as i64 + 1) & 0xFF;
        let (mut ytop, mut ybot) = (192i32, 0i32);
        for x in &v[..n] {
            let (px, py, pz, w) = (x.p[0], x.p[1], x.p[2], x.p[3]);
            let wl = w as i64;
            let mut g = GxVertex { x: px, y: py, z: pz, w, ..Default::default() };
            if wl > 0 {
                g.ix = ((px as i64 + wl) * vw / (wl * 2) + vx) as i16;
                g.iy = ((-(py as i64) + wl) * vh / (wl * 2) + vy) as i16;
                g.sx = ((px as f64 + wl as f64) * vw as f64 / (wl as f64 * 2.0) + vx as f64) as f32;
                g.sy = ((-(py as f64) + wl as f64) * vh as f64 / (wl as f64 * 2.0) + vy as f64) as f32;
            } else {
                g.ix = vx as i16;
                g.iy = vy as i16;
                g.sx = vx as f32;
                g.sy = vy as f32;
            }
            g.r = x.c[0] as f32 / 16.0;
            g.g = x.c[1] as f32 / 16.0;
            g.b = x.c[2] as f32 / 16.0;
            g.s = x.t[0] as f32 / 16.0;
            g.t = x.t[1] as f32 / 16.0;
            ytop = ytop.min(g.iy as i32);
            ybot = ybot.max(g.iy as i32);
            self.build.verts.push(g);
        }
        let (ytop, ybot) = (ytop.clamp(0, 255) as u32, ybot.clamp(0, 255) as u32);
        poly.sortkey = ybot << 8 | ytop;
        self.build.polys.push(poly);
        if self.unbounded {
            return false;
        }
        if let Some(s) = self.build.srcs.last_mut() {
            s.1 += 1;
        }
        self.build.nverts_ds += n as u32;
        false
    }
}

impl Nds {
    fn gx_submit(&mut self, v: &[V], n: usize) {
        if self.gx.submit_poly(v, n) {
            let d = self.io_reg16(9, 0x0400_0060) | 0x2000;
            self.io_set16(9, 0x0400_0060, d);
            self.once("gx", 1, || "3D: the polygon or vertex list is full (DS limits): polygons dropped".to_string());
        }
    }

    /// BEGIN_VTXS with a swapped texture: skip its vertices, and once a frame draw the file's model.
    /// Whether the batch that begins has the texture the swap is for.
    fn swap_matches(&mut self) -> u8 {
        let (tex, pal) = (self.gx.tex_param, self.gx.pltt_base);
        let Some(sw) = &self.gx.swap else { return 0 };
        let Some(want) = sw.key else { return (tex == sw.tex) as u8 };
        let wants: Vec<super::render3d::TexKey> = std::iter::once(want).chain(sw.hide.iter().copied()).collect();
        let gen = self.tex_generation();
        if self.gx.swap_gen != gen {
            self.gx.swap_cache.clear();
            self.gx.swap_gen = gen;
        }
        if let Some(&b) = self.gx.swap_cache.get(&(tex, pal)) {
            return b;
        }
        let mut r = 0;
        if wants.iter().any(|w| (tex >> 26) & 7 == w.fmt as u32 && super::texpack::dims(tex) == (w.w as usize, w.h as usize)) {
            let (tex_mem, pal_mem) = (self.vram_copy(super::video::VRAM_TEX, 0x80000), self.vram_copy(super::video::VRAM_TEXPAL, 0x18000));
            // the palette's hash is not compared: the same picture may be read through another palette
            let got = super::texpack::texture_key(&super::render3d::Mem { tex: &tex_mem, pal: &pal_mem }, tex, pal);
            r = match wants.iter().position(|w| w.data == got.data) {
                Some(0) => 1,
                Some(_) => 2,
                None => 0,
            };
        }
        self.gx.swap_cache.insert((tex, pal), r);
        r
    }

    /// BEGIN_VTXS: is the batch a replaced model's?
    fn ult_begin(&mut self) {
        if self.gx.ult.is_empty() {
            return;
        }
        self.ult_pend_close();
        self.gx.ult_close();
        let (src, now) = (self.gx.cmd_src, self.gx.swaps);
        if src == 0 {
            // not by DMA: known, if at all, by the words that follow
            let g = &mut self.gx;
            if !g.swap_pending && !g.build.srcs.is_empty() {
                g.update_clip();
                g.ult_pend = Some(UltPend { start: g.build.polys.len(), src_at: g.build.srcs.len() - 1, m: g.mtx_clip, attr: g.attr_pending, words: Vec::new() });
            }
            return;
        }
        let mut files = std::mem::take(&mut self.gx.files);
        let found = self.model_of(&mut files, src, now).and_then(|(fi, mesh)| self.gx.ult_of(&mut files, fi).map(|u| (u, mesh)));
        self.gx.files = files;
        let Some((u, mesh)) = found else { return };
        let g = &mut self.gx;
        if g.ult[u].only_mesh && g.ult[u].anchor != Some(mesh) {
            return;
        }
        g.update_clip();
        let (m, attr) = (g.mtx_clip, g.attr_pending);
        let i = g.ult_seen_at(u, mesh, m, attr);
        g.ult_open = Some((g.build.polys.len(), i));
    }

    /// The batch that came by the CPU's copy has ended: whose it is, by its words.
    fn ult_pend_close(&mut self) {
        let Some(p) = self.gx.ult_pend.take() else { return };
        let n = self.gx.build.polys.len();
        if n <= p.start {
            return;
        }
        let now = self.gx.swaps;
        let mut files = std::mem::take(&mut self.gx.files);
        let hit = self.model_by_words(&mut files, &p.words, now);
        let found = hit.and_then(|(fi, mesh)| self.gx.ult_of(&mut files, fi).map(|u| (u, mesh)));
        if let Some((fi, mesh)) = hit {
            // the frame's sources name the mesh's list from now on (`batches`)
            if let Some(s) = self.gx.build.srcs.get_mut(p.src_at) {
                s.0 = files.mesh_list(fi, mesh);
            }
        }
        self.gx.files = files;
        let Some((u, mesh)) = found else { return };
        let g = &mut self.gx;
        if g.ult[u].only_mesh && g.ult[u].anchor != Some(mesh) {
            return;
        }
        let i = g.ult_seen_at(u, mesh, p.m, p.attr);
        g.ult_seen[i].4 += n - p.start;
        g.ult_hide.push((p.start, n));
    }

    /// The VBlank of a swap: the frame's replaced batches go with its lists.
    fn ult_swap(&mut self) {
        self.ult_pend_close();
        let g = &mut self.gx;
        g.ult_close();
        g.ult_hide_r = std::mem::take(&mut g.ult_hide);
        g.ult_at_r.clear();
        for u in 0..g.ult.len() {
            // the anchor mesh's matrix (of its batches, the one with most polygons); with no anchor named, the mesh with most
            let anchor = g.ult[u].anchor;
            // (a named anchor's matrix holds even when all its polygons were clipped away)
            let best = g.ult_seen.iter().filter(|c| c.0 == u && anchor.map_or(c.4 > 0, |a| a == c.1)).max_by_key(|c| c.4);
            if let Some(c) = best {
                g.ult_at_r.push((u, c.2, c.3));
            }
        }
        let Gx { ult, ult_at_r, .. } = &mut *g;
        ult_once(ult_at_r, |c| ult[c.0].source);
        g.ult_seen.clear();
    }

    /// At the start of rendering: the replaced models' polygons out, the replacements in.
    fn ult_apply(&mut self, draw: bool) {
        if !draw {
            return; // kept: the game may not swap before the frame that is drawn
        }
        let g = &mut self.gx;
        let hide = std::mem::take(&mut g.ult_hide_r);
        let at = std::mem::take(&mut g.ult_at_r);
        if hide.is_empty() || at.is_empty() {
            return;
        }
        let mut i = 0;
        g.render.polys.retain(|_| {
            i += 1;
            !hide.iter().any(|&(a, b)| i > a && i - 1 < b)
        });
        ult_undim(&mut g.render);
        let saved = (g.attr, g.tex_param, g.pltt_base);
        std::mem::swap(&mut g.render, &mut g.build);
        g.unbounded = true;
        let pack = self.texpack.pack.clone();
        for (u, m, attr) in at {
            // opaque, both faces (the model may wind either way), the picture times the vertex colour
            self.gx.attr = (attr | 0xC0) & !(0x1F << 16 | 3 << 4) | 31 << 16;
            let parts = std::mem::take(&mut self.gx.ult[u].parts);
            for part in &parts {
                self.gx.attr = self.gx.attr & !(0x1F << 16) | part.alpha << 16;
                // a picture the pack does not have: the vertices' colours alone
                let has = part.key.is_some_and(|k| pack.get(&k).is_some());
                (self.gx.tex_param, self.gx.pltt_base) = if has { (part.tex, part.pal) } else { (0, 0) };
                for t in &part.tris {
                    let v = t.map(|(q, c, uv)| {
                        let mut nv = V::default();
                        for i in 0..4 {
                            nv.p[i] = (q[0] * m[i] as f64 + q[1] * m[4 + i] as f64 + q[2] * m[8 + i] as f64 + m[12 + i] as f64).round() as i32;
                        }
                        nv.c = c.map(|c| c << 4 | if c != 0 { 0xF } else { 0 });
                        nv.t = uv;
                        nv
                    });
                    self.gx_submit(&v, 3);
                }
            }
            self.gx.ult[u].parts = parts;
        }
        let g = &mut self.gx;
        g.unbounded = false;
        std::mem::swap(&mut g.render, &mut g.build);
        (g.attr, g.tex_param, g.pltt_base) = saved;
    }

    fn swap_begin(&mut self) {
        let g = &mut self.gx;
        g.swap_skip = false;
        if g.swaps.is_multiple_of(30) {
            if let Some(path) = std::env::var_os("SUNPETAL_MODEL_SWAP") {
                let mt = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                if mt.is_some() && mt != g.swap_mtime {
                    g.swap_mtime = mt;
                    if let Some(n) = ModelSwap::from_env() {
                        eprintln!("swap: reloaded, {} vertices", n.verts.len());
                        g.swap = Some(n);
                    }
                }
            }
        }
        let g = &mut self.gx;
        g.swap_close();
        let hit = self.swap_matches();
        let g = &mut self.gx;
        if g.swap.is_none() || hit == 0 {
            return;
        }
        // the batch is built as the game sends it (the counters it reads stay the same); its
        // polygons are taken out at the start of rendering, and the file's model goes in
        g.swap_open = Some(g.build.polys.len());
        if hit == 2 {
            return;
        }
        g.update_clip();
        let (m, attr, tex, pal) = (g.mtx_clip, g.attr_pending, g.tex_param, g.pltt_base);
        if let Some(sw) = &mut g.swap {
            // the part of the model with most polygons, by the clip matrix, is what the swap is drawn through
            let i = match sw.at.iter().position(|c| c.0 == m) {
                Some(i) => i,
                None => {
                    sw.at.push((m, attr, tex, pal, 0));
                    sw.at.len() - 1
                }
            };
            g.swap_cur = Some(i);
        }
    }

    /// At the start of rendering: the polygons of the swapped batches out, the model in.
    fn swap_apply(&mut self, draw: bool) {
        let g = &mut self.gx;
        let hide = std::mem::take(&mut g.swap_hide_r);
        let at = g.swap_at_r.take();
        let Some(sw) = &mut g.swap else { return };
        let Some((m, attr, tex, pal)) = at else { return };
        if hide.is_empty() || !draw {
            return;
        }
        let (e, size) = (sw.e, sw.size);
        let mut i = 0;
        g.render.polys.retain(|_| {
            i += 1;
            !hide.iter().any(|&(a, b)| i > a && i - 1 < b)
        });
        let mut out = Vec::with_capacity(sw.verts.len());
        for v in &sw.verts {
            let mut q = [0f64; 3];
            for (i, qi) in q.iter_mut().enumerate() {
                *qi = v[0] * e[i] + v[1] * e[3 + i] + v[2] * e[6 + i] + e[9 + i];
            }
            let mut nv = V::default();
            for i in 0..4 {
                nv.p[i] = (q[0] * m[i] as f64 + q[1] * m[4 + i] as f64 + q[2] * m[8 + i] as f64 + m[12 + i] as f64).round() as i32;
            }
            for k in 0..3 {
                let c = ((v[5 + k] * 31.0 / 128.0).round() as i32).clamp(0, 31);
                nv.c[k] = c << 4 | if c != 0 { 0xF } else { 0 };
            }
            nv.t = [(v[3] * size * 16.0).round() as i32, (v[4] * size * 16.0).round() as i32];
            out.push(nv);
        }
        let saved = (g.attr, g.tex_param, g.pltt_base);
        // both faces: the file's model may wind either way; into the frame's list, not the one being built
        (g.attr, g.tex_param, g.pltt_base) = (attr | 0xC0, tex, pal);
        std::mem::swap(&mut g.render, &mut g.build);
        for t in out.as_chunks::<3>().0 {
            self.gx_submit(t, 3);
        }
        std::mem::swap(&mut self.gx.render, &mut self.gx.build);
        (self.gx.attr, self.gx.tex_param, self.gx.pltt_base) = saved;
    }

    fn gx_add_vertex(&mut self) {
        let g = &mut self.gx;
        if g.swap.is_some() && g.prim >= 0 {
            *g.swap_count.entry(g.tex_param).or_insert(0) += 1;
        }
        if g.prim < 0 || g.swap_skip {
            return;
        }
        if g.tex_param >> 30 == 3 {
            // texture coordinates from the vertex
            let m = g.mtx_tex;
            for i in 0..2 {
                g.tc[i] = ((g.vtx[0] as i64 * m[i] as i64 + g.vtx[1] as i64 * m[4 + i] as i64 + g.vtx[2] as i64 * m[8 + i] as i64 + ((g.raw_tc[i] as i64) << 24)) >> 24) as i16;
            }
        }
        #[cfg(feature = "debug-tools")]
        {
            let (frame, v, tex, pos) = (g.swaps, g.vtx, g.tex_param, g.mtx_pos);
            if let Some(t) = &mut g.trace {
                t.vertex(frame, v, &pos, tex);
            }
        }
        g.update_clip();
        let m = g.mtx_clip;
        let mut nv = V::default();
        for i in 0..4 {
            nv.p[i] = ((g.vtx[0] as i64 * m[i] as i64 + g.vtx[1] as i64 * m[4 + i] as i64 + g.vtx[2] as i64 * m[8 + i] as i64 + 0x1000 * m[12 + i] as i64) >> 12) as i32;
        }
        for i in 0..3 {
            let c = g.vcolor[i] as i32;
            nv.c[i] = c << 4 | if c != 0 { 0xF } else { 0 }; // 9 bits, as the DS keeps them
        }
        nv.t = [g.tc[0] as i32, g.tc[1] as i32];
        g.strip_count += 1;
        let strip = &mut g.strip;
        match g.prim {
            0 => {
                // triangles
                strip[g.nstrip] = nv;
                g.nstrip += 1;
                if g.nstrip == 3 {
                    let s = *strip;
                    g.nstrip = 0;
                    self.gx_submit(&s, 3);
                }
            }
            1 => {
                // quads
                strip[g.nstrip] = nv;
                g.nstrip += 1;
                if g.nstrip == 4 {
                    let s = *strip;
                    g.nstrip = 0;
                    self.gx_submit(&s, 4);
                }
            }
            2 => {
                // triangle strip: every other one turned round
                if g.nstrip < 2 {
                    strip[g.nstrip] = nv;
                    g.nstrip += 1;
                    return;
                }
                strip[2] = nv;
                let odd = g.strip_count & 1 != 0;
                let t = if odd { [strip[0], strip[1], strip[2]] } else { [strip[1], strip[0], strip[2]] };
                strip[0] = strip[1];
                strip[1] = strip[2];
                self.gx_submit(&t, 3);
            }
            3 => {
                // quad strip: 0 1 3 2, then on in pairs
                if g.nstrip < 3 {
                    strip[g.nstrip] = nv;
                    g.nstrip += 1;
                    return;
                }
                strip[3] = nv;
                let q = [strip[0], strip[1], strip[3], strip[2]];
                strip[0] = strip[2];
                strip[1] = strip[3];
                g.nstrip = 2;
                self.gx_submit(&q, 4);
            }
            _ => {}
        }
    }

    // ---- the tests ---------------------------------------------------------------------------
    fn gx_box_test(&mut self, p: &[u32]) {
        let (x, y, z) = (p[0] as i16 as i32, (p[0] >> 16) as i16 as i32, p[1] as i16 as i32);
        let (w, h, d) = ((p[1] >> 16) as i16 as i32, p[2] as i16 as i32, (p[2] >> 16) as i16 as i32);
        let g = &mut self.gx;
        g.update_clip();
        let m = g.mtx_clip;
        let mut c = [V::default(); 8];
        for (i, ci) in c.iter_mut().enumerate() {
            let px = x + if i & 1 != 0 { w } else { 0 };
            let py = y + if i & 2 != 0 { h } else { 0 };
            let pz = z + if i & 4 != 0 { d } else { 0 };
            for k in 0..4 {
                ci.p[k] = ((px as i64 * m[k] as i64 + py as i64 * m[4 + k] as i64 + pz as i64 * m[8 + k] as i64 + 0x1000 * m[12 + k] as i64) >> 12) as i32;
            }
        }
        const FACES: [[usize; 4]; 6] = [[0, 1, 3, 2], [4, 5, 7, 6], [0, 1, 5, 4], [2, 3, 7, 6], [0, 2, 6, 4], [1, 3, 7, 5]];
        let mut inside = false;
        for f in FACES.iter() {
            if inside {
                break;
            }
            let mut v = [V::default(); 10];
            for k in 0..4 {
                v[k] = c[f[k]];
            }
            let mut n = clip_axis(&mut v, 4, 2);
            if n != 0 {
                n = clip_axis(&mut v, n, 0);
            }
            if n != 0 {
                n = clip_axis(&mut v, n, 1);
            }
            inside = n > 0;
        }
        g.stat = (g.stat & !2) | if inside { 2 } else { 0 };
    }

    // ---- the commands ------------------------------------------------------------------------
    fn gx_exec(&mut self, cmd: u32, p: &[u32]) {
        match cmd {
            0x00 => {}
            0x10 => self.gx.mtx_mode = p[0] & 3,
            0x11 => {
                // MTX_PUSH
                let g = &mut self.gx;
                if g.mtx_mode == 0 {
                    if g.sp_proj != 0 {
                        g.stat |= 0x8000;
                    }
                    g.stk_proj = g.mtx_proj;
                    g.sp_proj = 1;
                } else if g.mtx_mode == 3 {
                    if g.sp_tex != 0 {
                        g.stat |= 0x8000;
                    }
                    g.stk_tex = g.mtx_tex;
                    g.sp_tex = 1;
                } else {
                    if g.sp_pos > 30 {
                        g.stat |= 0x8000;
                    }
                    let i = (g.sp_pos & 31) as usize;
                    g.stk_pos[i] = g.mtx_pos;
                    g.stk_vec[i] = g.mtx_vec;
                    g.sp_pos = (g.sp_pos + 1) & 0x3F;
                }
            }
            0x12 => {
                // MTX_POP
                let g = &mut self.gx;
                if g.mtx_mode == 0 {
                    if g.sp_proj == 0 {
                        g.stat |= 0x8000;
                    }
                    g.sp_proj = 0;
                    g.mtx_proj = g.stk_proj;
                    g.clip_dirty = true;
                } else if g.mtx_mode == 3 {
                    if g.sp_tex == 0 {
                        g.stat |= 0x8000;
                    }
                    g.sp_tex = 0;
                    g.mtx_tex = g.stk_tex;
                } else {
                    let off = ((p[0] << 26) as i32) >> 26;
                    g.sp_pos = (g.sp_pos as i32 - off) as u32 & 0x3F;
                    if g.sp_pos > 30 {
                        g.stat |= 0x8000;
                    }
                    let i = (g.sp_pos & 31) as usize;
                    g.mtx_pos = g.stk_pos[i];
                    g.mtx_vec = g.stk_vec[i];
                    g.clip_dirty = true;
                }
            }
            0x13 => {
                // MTX_STORE
                let g = &mut self.gx;
                if g.mtx_mode == 0 {
                    g.stk_proj = g.mtx_proj;
                } else if g.mtx_mode == 3 {
                    g.stk_tex = g.mtx_tex;
                } else {
                    let a = (p[0] & 31) as usize;
                    if a == 31 {
                        g.stat |= 0x8000;
                    }
                    g.stk_pos[a] = g.mtx_pos;
                    g.stk_vec[a] = g.mtx_vec;
                }
            }
            0x14 => {
                // MTX_RESTORE
                let g = &mut self.gx;
                if g.mtx_mode == 0 {
                    g.mtx_proj = g.stk_proj;
                    g.clip_dirty = true;
                } else if g.mtx_mode == 3 {
                    g.mtx_tex = g.stk_tex;
                } else {
                    let a = (p[0] & 31) as usize;
                    if a == 31 {
                        g.stat |= 0x8000;
                    }
                    g.mtx_pos = g.stk_pos[a];
                    g.mtx_vec = g.stk_vec[a];
                    g.last_restore = a as u8;
                    g.clip_dirty = true;
                }
            }
            0x15 => self.gx.on_current(true, |m| *m = IDENTITY),
            0x16 => {
                let q: Vec<i32> = p[..16].iter().map(|x| *x as i32).collect();
                self.gx.on_current(true, |m| m.copy_from_slice(&q));
            }
            0x17 => {
                let q: Vec<i32> = p[..12].iter().map(|x| *x as i32).collect();
                self.gx.on_current(true, |m| {
                    for r in 0..4 {
                        for c in 0..3 {
                            m[r * 4 + c] = q[r * 3 + c];
                        }
                        m[r * 4 + 3] = if r == 3 { 0x1000 } else { 0 };
                    }
                });
            }
            0x18 => {
                let mut n: Mtx = [0; 16];
                for i in 0..16 {
                    n[i] = p[i] as i32;
                }
                self.gx.on_current(true, |m| mult44(m, &n));
            }
            0x19 => {
                let q: Vec<i32> = p[..12].iter().map(|x| *x as i32).collect();
                self.gx.on_current(true, |m| mult43(m, &q));
            }
            0x1A => {
                let q: Vec<i32> = p[..9].iter().map(|x| *x as i32).collect();
                self.gx.on_current(true, |m| mult33(m, &q));
            }
            0x1B => {
                let q: Vec<i32> = p[..3].iter().map(|x| *x as i32).collect();
                self.gx.on_current(false, |m| scale(m, &q));
            }
            0x1C => {
                let q: Vec<i32> = p[..3].iter().map(|x| *x as i32).collect();
                self.gx.on_current(true, |m| translate(m, &q));
            }
            0x20 => {
                let g = &mut self.gx;
                g.vcolor = [(p[0] & 31) as u8, ((p[0] >> 5) & 31) as u8, ((p[0] >> 10) & 31) as u8];
            }
            0x21 => {
                // NORMAL
                let g = &mut self.gx;
                for i in 0..3 {
                    g.normal[i] = (((p[0] >> (10 * i) << 22) as i32) >> 22) as i16;
                }
                if g.tex_param >> 30 == 2 {
                    let m = g.mtx_tex;
                    for i in 0..2 {
                        g.tc[i] = ((((g.normal[0] as i64) << 3) * m[i] as i64
                            + ((g.normal[1] as i64) << 3) * m[4 + i] as i64
                            + ((g.normal[2] as i64) << 3) * m[8 + i] as i64
                            + ((g.raw_tc[i] as i64) << 24))
                            >> 24) as i16;
                    }
                }
                g.lighting();
            }
            0x22 => {
                // TEXCOORD
                let g = &mut self.gx;
                g.raw_tc = [p[0] as i16, (p[0] >> 16) as i16];
                if g.tex_param >> 30 == 1 {
                    let m = g.mtx_tex;
                    for i in 0..2 {
                        g.tc[i] = ((g.raw_tc[0] as i64 * m[i] as i64 + g.raw_tc[1] as i64 * m[4 + i] as i64 + m[8 + i] as i64 + m[12 + i] as i64) >> 12) as i16;
                    }
                } else {
                    g.tc = g.raw_tc;
                }
            }
            0x23 => {
                self.gx.vtx = [p[0] as i16, (p[0] >> 16) as i16, p[1] as i16];
                self.gx_add_vertex();
            }
            0x24 => {
                for i in 0..3 {
                    self.gx.vtx[i] = ((((p[0] >> (10 * i)) & 0x3FF) << 6) as u16) as i16;
                }
                self.gx_add_vertex();
            }
            0x25 => {
                self.gx.vtx[0] = p[0] as i16;
                self.gx.vtx[1] = (p[0] >> 16) as i16;
                self.gx_add_vertex();
            }
            0x26 => {
                self.gx.vtx[0] = p[0] as i16;
                self.gx.vtx[2] = (p[0] >> 16) as i16;
                self.gx_add_vertex();
            }
            0x27 => {
                self.gx.vtx[1] = p[0] as i16;
                self.gx.vtx[2] = (p[0] >> 16) as i16;
                self.gx_add_vertex();
            }
            0x28 => {
                for i in 0..3 {
                    let d = ((p[0] >> (10 * i) << 22) as i32) >> 22;
                    self.gx.vtx[i] = (self.gx.vtx[i] as i32 + d) as i16;
                }
                self.gx_add_vertex();
            }
            0x29 => self.gx.attr_pending = p[0],
            0x2A => self.gx.tex_param = p[0],
            0x2B => self.gx.pltt_base = p[0] & 0x1FFF,
            0x30 => {
                let g = &mut self.gx;
                g.diffuse = [(p[0] & 31) as u8, ((p[0] >> 5) & 31) as u8, ((p[0] >> 10) & 31) as u8];
                g.ambient = [((p[0] >> 16) & 31) as u8, ((p[0] >> 21) & 31) as u8, ((p[0] >> 26) & 31) as u8];
                if p[0] & 0x8000 != 0 {
                    g.vcolor = g.diffuse;
                }
            }
            0x31 => {
                let g = &mut self.gx;
                g.specular = [(p[0] & 31) as u8, ((p[0] >> 5) & 31) as u8, ((p[0] >> 10) & 31) as u8];
                g.emission = [((p[0] >> 16) & 31) as u8, ((p[0] >> 21) & 31) as u8, ((p[0] >> 26) & 31) as u8];
                g.shine_table_on = p[0] & 0x8000 != 0;
            }
            0x32 => {
                // LIGHT_VECTOR
                let g = &mut self.gx;
                let l = (p[0] >> 30) as usize;
                let mut d = [0i32; 3];
                for i in 0..3 {
                    d[i] = ((p[0] >> (10 * i) << 22) as i32) >> 22;
                }
                let m = g.mtx_vec;
                for i in 0..3 {
                    g.light_vec[l][i] = ((d[0] as i64 * m[i] as i64 + d[1] as i64 * m[4 + i] as i64 + d[2] as i64 * m[8 + i] as i64) >> 12) as i32;
                }
            }
            0x33 => {
                let g = &mut self.gx;
                let l = (p[0] >> 30) as usize;
                g.light_col[l] = [(p[0] & 31) as u8, ((p[0] >> 5) & 31) as u8, ((p[0] >> 10) & 31) as u8];
            }
            0x34 => {
                for i in 0..32 {
                    for k in 0..4 {
                        self.gx.shine_table[i * 4 + k] = (p[i] >> (8 * k)) as u8;
                    }
                }
            }
            0x40 => {
                // BEGIN_VTXS
                let g = &mut self.gx;
                if g.build.srcs.last().map(|s| (s.0, s.2)) != Some((g.cmd_src, g.last_restore)) {
                    g.build.srcs.push((g.cmd_src, 0, g.last_restore));
                }
                #[cfg(feature = "debug-tools")]
                {
                    let (frame, pos, tex, pal) = (g.swaps, g.mtx_pos, g.tex_param, g.pltt_base);
                    if let Some(t) = &mut g.trace {
                        t.flush(frame);
                        if frame >= t.lo && frame <= t.hi {
                            t.used.insert((tex, pal));
                            t.open = Some(((p[0] & 3) as i32, 0, [i16::MAX; 3], [i16::MIN; 3], pos, tex));
                        }
                    }
                }
                g.prim = (p[0] & 3) as i32;
                g.attr = g.attr_pending;
                g.nstrip = 0;
                g.strip_count = 0;
                self.swap_begin();
                self.ult_begin();
            }
            0x41 => {} // END_VTXS: nothing on the DS either
            0x50 => {
                // SWAP_BUFFERS: at the VBlank
                #[cfg(feature = "debug-tools")]
                {
                    let frame = self.gx.swaps;
                    let mut used = Vec::new();
                    if let Some(t) = &mut self.gx.trace {
                        t.flush(frame);
                        t.poll(frame);
                        used = std::mem::take(&mut t.used).into_iter().collect();
                    }
                    if !used.is_empty() {
                        let (tex_mem, pal_mem) = (self.vram_copy(super::video::VRAM_TEX, 0x80000), self.vram_copy(super::video::VRAM_TEXPAL, 0x18000));
                        let m = super::render3d::Mem { tex: &tex_mem, pal: &pal_mem };
                        used.sort();
                        if let Some(t) = &mut self.gx.trace {
                            use std::io::Write;
                            for (tex, pal) in used {
                                if (tex >> 26) & 7 == 0 {
                                    continue;
                                }
                                let key = super::texpack::file_name(&super::texpack::texture_key(&m, tex, pal));
                                let _ = writeln!(t.out, "K f={frame} tex={tex:#010x} pal={pal:#x} key={}", key.trim_end_matches(".png"));
                            }
                        }
                    }
                }
                self.gx.swap_pending = true;
                self.gx.swap_param = p[0];
            }
            0x60 => {
                // VIEWPORT (y counted from the bottom)
                let (x1, y1, x2, y2) = (p[0] as u8, (p[0] >> 8) as u8, (p[0] >> 16) as u8, (p[0] >> 24) as u8);
                self.gx.viewport = [x1, 191u8.wrapping_sub(y2), x2, 191u8.wrapping_sub(y1)];
            }
            0x70 => self.gx_box_test(p),
            0x71 => {
                // POS_TEST
                let g = &mut self.gx;
                g.vtx = [p[0] as i16, (p[0] >> 16) as i16, p[1] as i16];
                g.update_clip();
                let m = g.mtx_clip;
                for i in 0..4 {
                    g.pos_result[i] =
                        ((g.vtx[0] as i64 * m[i] as i64 + g.vtx[1] as i64 * m[4 + i] as i64 + g.vtx[2] as i64 * m[8 + i] as i64 + 0x1000 * m[12 + i] as i64) >> 12) as i32;
                }
            }
            0x72 => {
                // VEC_TEST
                let g = &mut self.gx;
                let mut n = [0i32; 3];
                for i in 0..3 {
                    n[i] = ((p[0] >> (10 * i) << 22) as i32) >> 22;
                }
                let m = g.mtx_vec;
                for i in 0..3 {
                    let r = ((n[0] as i64 * m[i] as i64 + n[1] as i64 * m[4 + i] as i64 + n[2] as i64 * m[8 + i] as i64) >> 9) as i32;
                    g.vec_result[i] = (if r & 0x1000 != 0 { r | 0xF000 } else { r & 0x0FFF }) as i16;
                }
            }
            _ => self.once("gx cmd", cmd, || format!("3D: command {:02X} not known", cmd)),
        }
    }

    /// A whole command: now, or behind a swap.
    fn gx_command(&mut self, cmd: u32, p: &[u32]) {
        if self.gx.swap_pending {
            let n = if cmd < 0x80 { PARAMS[cmd as usize] as usize } else { 0 };
            self.gx.queue.push_back(Queued { cmd, p: p[..n].to_vec(), src: self.gx.dl_src });
            return;
        }
        self.gx.cmd_src = self.gx.dl_src;
        self.gx_exec(cmd, p);
    }

    fn gx_fifo_word(&mut self, v: u32) {
        if let Some(p) = &mut self.gx.ult_pend {
            if p.words.len() < 6 {
                p.words.push(v);
            }
        }
        if self.gx.cur >= 0 {
            // a parameter of the command being gathered
            let g = &mut self.gx;
            g.params[g.got] = v;
            g.got += 1;
            if g.got < g.need {
                return;
            }
            let (cmd, params) = (g.cur as u32, g.params);
            g.cur = -1;
            self.gx_command(cmd, &params);
        } else {
            self.gx.cmds = v;
            self.gx.ncmds = 4;
        }
        // the packed commands that need no parameters run now; the next one with parameters waits
        while self.gx.ncmds > 0 {
            let c = self.gx.cmds & 0xFF;
            self.gx.cmds >>= 8;
            self.gx.ncmds -= 1;
            if !known(c) {
                self.once("gx cmd", c, || format!("3D: packed command {:02X} not known", c));
            }
            let need = if c < 0x80 { PARAMS[c as usize] as usize } else { 0 };
            if need == 0 {
                if c != 0 {
                    let params = self.gx.params;
                    self.gx_command(c, &params);
                }
                if self.gx.cmds == 0 {
                    self.gx.ncmds = 0; // only NOPs left in the word
                }
                continue;
            }
            self.gx.cur = c as i32;
            self.gx.need = need;
            self.gx.got = 0;
            return;
        }
    }

    fn gx_port_write(&mut self, cmd: u32, v: u32) {
        let need = PARAMS[cmd as usize & 0x7F] as usize;
        let c = cmd as usize & 0x7F;
        if need == 0 {
            let p = self.gx.port_params[c];
            return self.gx_command(cmd, &p);
        }
        let g = &mut self.gx;
        g.port_params[c][g.port_got[c]] = v;
        g.port_got[c] += 1;
        if g.port_got[c] < need {
            return;
        }
        g.port_got[c] = 0;
        let p = g.port_params[c];
        self.gx_command(cmd, &p);
    }

    // ---- the registers -----------------------------------------------------------------------
    fn gxstat(&self) -> u32 {
        let g = &self.gx;
        let mut v = g.stat & 0xC000_8002;
        v |= (g.sp_pos & 31) << 8 | (g.sp_proj & 1) << 13;
        let n = g.queue.len().min(256) as u32;
        v |= n << 16;
        if n < 128 {
            v |= 1 << 25;
        }
        if n == 0 {
            v |= 1 << 26;
        }
        if g.swap_pending {
            v |= 1 << 27;
        }
        v
    }

    fn gx_fifo_irq(&mut self) {
        let (m, n) = (self.gx.stat >> 30, self.gx.queue.len());
        if (m == 1 && n < 128) || (m == 2 && n == 0) {
            self.irq_raise(9, 21);
        }
    }

    pub(super) fn gx_write(&mut self, a: u32, v: u32, size: usize) {
        let off = a - 0x0400_0320;
        if (0x0400_0400..0x0400_0440).contains(&a) {
            if size != 4 {
                self.once("gx", 2, || format!("3D: a {}-byte write to GXFIFO", size));
            }
            return self.gx_fifo_word(v);
        }
        if (0x0400_0440..0x0400_0600).contains(&a) {
            if size != 4 {
                self.once("gx", 3, || format!("3D: a {}-byte write to a command port", size));
            }
            return self.gx_port_write((a - 0x0400_0400) >> 2, v);
        }
        if a == 0x0400_0600 || a == 0x0400_0602 {
            // GXSTAT: the FIFO interrupt's mode, the error bit acknowledged
            let w = if a == 0x0400_0602 && size == 2 { v << 16 } else { v };
            if size == 4 || a == 0x0400_0602 {
                self.gx.stat = (self.gx.stat & !0xC000_0000) | (w & 0xC000_0000);
            }
            if (size == 4 || a == 0x0400_0600) && w & 0x8000 != 0 {
                self.gx.stat &= !0x8000;
                self.gx.sp_proj = 0;
                self.gx.sp_tex = 0;
            }
            return self.gx_fifo_irq();
        }
        let o = off as usize;
        if o + size <= self.gx.regs.len() {
            for i in 0..size {
                self.gx.regs[o + i] = (v >> (8 * i)) as u8;
            }
        }
    }

    pub(super) fn gx_read(&mut self, a: u32, _size: usize) -> u32 {
        let off = a & !3;
        let g = &mut self.gx;
        if off == 0x0400_0600 {
            return self.gxstat();
        }
        if off == 0x0400_0604 {
            return g.build.polys.len() as u32 | g.build.nverts_ds << 16;
        }
        if (0x0400_0620..0x0400_0630).contains(&off) {
            return g.pos_result[((off - 0x0400_0620) >> 2) as usize] as u32;
        }
        if (0x0400_0630..0x0400_0636).contains(&off) {
            let i = ((off - 0x0400_0630) >> 1) as usize;
            return g.vec_result[i] as u16 as u32 | if i < 2 { (g.vec_result[i + 1] as u16 as u32) << 16 } else { 0 };
        }
        if (0x0400_0640..0x0400_0680).contains(&off) {
            #[cfg(feature = "debug-tools")]
            super::debug::note_clip_read();
            g.update_clip();
            return g.mtx_clip[((off - 0x0400_0640) >> 2) as usize] as u32;
        }
        if (0x0400_0680..0x0400_06A4).contains(&off) {
            let i = ((off - 0x0400_0680) >> 2) as usize;
            return g.mtx_vec[(i / 3) * 4 + i % 3] as u32;
        }
        let o = (off - 0x0400_0320) as usize;
        if o + 4 <= g.regs.len() {
            return u32::from_le_bytes([g.regs[o], g.regs[o + 1], g.regs[o + 2], g.regs[o + 3]]);
        }
        0
    }

    // ---- the swap, and the rendering engine's job ----------------------------------------------
    /// At the VBlank: a SWAP_BUFFERS waiting sends the lists to the rendering engine and the
    /// queue behind it runs.
    pub(super) fn gx_vblank(&mut self) {
        if !self.gx.swap_pending {
            return;
        }
        let g = &mut self.gx;
        g.swap_pending = false;
        // the swapped batches of the frame that goes to the rendering engine
        self.ult_swap();
        let g = &mut self.gx;
        g.swap_close();
        g.swap_hide_r = std::mem::take(&mut g.swap_hide);
        g.swap_at_r = g.swap.as_mut().and_then(|sw| {
            let best = sw.at.iter().max_by_key(|c| c.4).map(|c| (c.0, c.1, c.2, c.3));
            sw.at.clear();
            best
        });
        std::mem::swap(&mut g.render, &mut g.build);
        g.build = Lists::default();
        g.render_manual = g.swap_param & 1 != 0;
        g.render_wbuf = g.swap_param & 2 != 0;
        g.swaps += 1;
        while !self.gx.queue.is_empty() && !self.gx.swap_pending {
            let q = self.gx.queue.pop_front().unwrap();
            self.gx.cmd_src = q.src;
            self.gx_exec(q.cmd, &q.p);
        }
        self.gx_fifo_irq();
    }

    pub(super) fn gx_line(&mut self) {
        self.gx_fifo_irq();
    }

    fn gx_r16(&self, a: u32) -> u16 {
        let o = (a - 0x0400_0320) as usize;
        u16::from_le_bytes([self.gx.regs[o], self.gx.regs[o + 1]])
    }

    /// The rendering engine starts a frame: its job is taken (lists, 3D registers, texture
    /// memory) and drawn, to be shown in the next frame.
    pub(super) fn gx_render_start(&mut self) {
        let t = std::time::Instant::now();
        self.ult_apply(self.gpu.on);
        self.swap_apply(self.gpu.on);
        let t = super::note_hw(0, t);
        if !self.gpu.on {
            return; // a headless run with no picture of the next frame
        }
        let g = &self.gx;
        let mut polys: Vec<GxPoly> = g.render.polys.iter().filter(|p| !p.translucent).copied().collect();
        let nopaque = polys.len();
        polys.extend(g.render.polys.iter().filter(|p| p.translucent).copied());
        // the order of drawing: opaque polygons, then translucent ones; each sorted by y unless
        // SWAP_BUFFERS asked for the translucent ones in their own order
        polys[..nopaque].sort_by_key(|p| p.sortkey);
        if !g.render_manual {
            polys[nopaque..].sort_by_key(|p| p.sortkey);
        }
        let mut fog_table = [0u8; 32];
        fog_table.copy_from_slice(&g.regs[0x360 - 0x320..0x380 - 0x320]);
        let mut toon = [0u16; 32];
        for (i, t) in toon.iter_mut().enumerate() {
            *t = self.gx_r16(0x0400_0380 + 2 * i as u32);
        }
        let mut edge = [0u16; 8];
        for (i, e) in edge.iter_mut().enumerate() {
            *e = self.gx_r16(0x0400_0330 + 2 * i as u32);
        }
        let t = super::note_hw(3, t);
        let (tex_mem, texpal_mem) = (self.vram_copy(super::video::VRAM_TEX, 0x80000), self.vram_copy(super::video::VRAM_TEXPAL, 0x18000));
        let t = super::note_hw(1, t);
        let texture_on = self.io_reg16(9, 0x0400_0060) & 1 != 0;
        let keys = self.texture_keys(&polys, &super::render3d::Mem { tex: &tex_mem, pal: &texpal_mem }, texture_on);
        let t = super::note_hw(2, t);
        let g = &self.gx;
        let reg32 = |o: usize| u32::from_le_bytes([g.regs[o], g.regs[o + 1], g.regs[o + 2], g.regs[o + 3]]);
        let job = Job {
            verts: g.render.verts.clone(),
            polys,
            wbuffer: g.render_wbuf,
            disp3dcnt: self.io_reg16(9, 0x0400_0060),
            width: 256 + 2 * g.wide,
            clear_color: reg32(0x350 - 0x320),
            clear_depth: self.gx_r16(0x0400_0354) & 0x7FFF,
            clrimage_offset: self.gx_r16(0x0400_0356),
            alpha_ref: g.regs[0x340 - 0x320] & 31,
            extras: self.extras,
            edge,
            fog_color: reg32(0x358 - 0x320),
            fog_offset: self.gx_r16(0x0400_035C),
            fog_table,
            toon,
            tex: tex_mem,
            texpal: texpal_mem,
            keys,
            pack: self.texpack.pack.clone(),
        };
        if std::env::var_os("SUNPETAL_3D_DEBUG").is_some() {
            eprintln!("3D frame: {} polys, wbuf {}, disp3dcnt {:04X}, clear {:08X}", job.polys.len(), job.wbuffer, job.disp3dcnt, job.clear_color);
            for p in job.polys.iter().take(std::env::var("SUNPETAL_3D_DEBUG").ok().and_then(|v| v.parse().ok()).unwrap_or(6)) {
                eprintln!("  poly attr {:08X} tex {:08X} n {}", p.attr, p.tex, p.n);
                for v in &job.verts[p.first as usize..(p.first + p.n) as usize] {
                    eprintln!("    clip {} {} {} {}  scr {:.1} {:.1}  rgb {:.1} {:.1} {:.1}  st {:.1} {:.1}", v.x, v.y, v.z, v.w, v.sx, v.sy, v.r, v.g, v.b, v.s, v.t);
                }
            }
        }
        let t = super::note_hw(3, t);
        self.settle_3d();
        let t = super::note_hw(4, t);
        let scale = self.scale;
        let mut drawing = None;
        let mut early = None;
        match self.renderer {
            #[cfg(feature = "gpu")]
            super::Renderer::Gpu if !job.polys.is_empty() => match super::render3d_gpu::begin(&job, scale) {
                Ok(d) => drawing = Some(d),
                Err(e) => self.gpu_failed(&e),
            },
            _ => {}
        }
        if drawing.is_none() {
            early = Some(super::render3d::render(&job, scale));
        }
        self.pending3d = Some(Pending3d { job, scale, vblank: self.vblanks, drawing, early });
        super::note_hw(5, t);
    }

    /// The GPU's failure told once per distinct message; the software engine takes over.
    #[cfg(feature = "gpu")]
    fn gpu_failed(&mut self, e: &str) {
        let mut h = 0u32;
        for b in e.bytes() {
            h = h.wrapping_mul(31).wrapping_add(b as u32);
        }
        self.once("3d-gpu", h, || format!("GPU renderer failed: {e}; the software one takes over"));
    }

    /// Finishes the frame the rendering engine started (`gx_render_start`): the GPU's picture is waited
    /// for and read, a failed one is drawn again by the software engine. `run_frame` calls it before it
    /// returns, so outside the emulation `frame3d` is always the last frame's (этап 9.2).
    /// A GPU drawing is in flight (not yet read).
    pub(super) fn pending_in_flight(&self) -> bool {
        self.pending3d.as_ref().is_some_and(|p| p.drawing.is_some())
    }

    pub(super) fn settle_3d(&mut self) {
        let Some(p) = self.pending3d.take() else { return };
        let Pending3d { job, scale, vblank, drawing, early } = p;
        let mut result = early;
        #[cfg(feature = "gpu")]
        if let Some(d) = drawing {
            result = Some(match super::render3d_gpu::end(d) {
                Ok(v) => v,
                Err(e) => {
                    self.gpu_failed(&e);
                    super::render3d::render(&job, scale)
                }
            });
        }
        #[cfg(not(feature = "gpu"))]
        let _ = drawing;
        let (px, notes) = result.expect("a frame is either drawn or given to the GPU");
        super::jobdump::maybe_dump(&job, scale, vblank, &px, notes);
        self.frame3d_wide = (job.width - 256) / 2;
        self.frame3d_scale = scale;
        self.frame3d = px;
        for (s, text) in super::render3d::NOTE.iter().enumerate() {
            if notes >> s & 1 != 0 {
                self.once("3d", s as u32, || text.to_string());
            }
        }
    }
}

/// A 3D frame the rendering engine has started and `settle_3d` has not yet finished: its job (for the
/// software engine to redraw it if the GPU fails), and either the GPU's drawing in flight or a picture
/// already made.
pub(super) struct Pending3d {
    job: super::render3d::Job,
    scale: usize,
    vblank: u64,
    #[cfg(feature = "gpu")]
    drawing: Option<super::render3d_gpu::Drawing>,
    #[cfg(not(feature = "gpu"))]
    drawing: Option<()>,
    early: Option<(Vec<u32>, u32)>,
}

/// Draws a frame with the GPU engine (`gpu`), except one with no polygons (the logos, a menu: only
/// the clear plane and the fog), which the software engine draws for next to nothing while the
/// GPU pays its fixed cost (≈ 0.7 ms: the frame's resources and the wait); the two agree word for
/// word on such frames (the parity dumps `logos_empty_*`). If the GPU fails (no device, a device error) the
/// software engine draws it instead and the failure comes back as text, for a note. The device is
/// made once and its failure is kept, so a machine with none pays only for the check each frame.
#[cfg(all(test, feature = "gpu"))]
fn gpu_or_software(job: &super::render3d::Job, scale: usize, gpu: impl FnOnce() -> Result<(Vec<u32>, u32), String>) -> ((Vec<u32>, u32), Option<String>) {
    if job.polys.is_empty() {
        return (super::render3d::render(job, scale), None);
    }
    match gpu() {
        Ok(v) => (v, None),
        Err(e) => (super::render3d::render(job, scale), Some(e)),
    }
}

#[cfg(test)]
mod ult_tests {
    #[test]
    fn see_through_materials_come_last_with_their_vertex_alpha() {
        use sunpetal_data::spm2::{Material, Model, Part, Vertex};
        let v = |a: u8| Vertex { pos: [0.0; 3], nrm: [0.0; 3], rgba: [128, 128, 128, a], uv: [0.0; 2], bone: 0 };
        let mat = |pass, blend| Material { picture: None, pass, blend, flags: 0 };
        // material 0 is see-through in pass 0, 1 is solid in pass 1, 2 has no state known (255)
        let m = Model {
            name: "t".into(),
            unit: 1.0,
            bind: Some(sunpetal_data::spm2::Bind { model: "x".into(), size: None, mesh: None, only_mesh: false, place: [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.] }),
            bones: vec![],
            materials: vec![mat(0, 2), mat(1, 0), mat(1, 255)],
            verts: vec![v(64), v(64), v(64), v(128), v(128), v(128)],
            parts: vec![Part { material: 0, first: 0, count: 3 }, Part { material: 1, first: 3, count: 3 }, Part { material: 2, first: 3, count: 3 }],
            idx: vec![0, 1, 2, 3, 4, 5],
        };
        let u = super::Ult::new(&m, &mut 0).unwrap();
        let alphas: Vec<u32> = u.parts.iter().map(|p| p.alpha).collect();
        assert_eq!(alphas, vec![31, 31, 15], "solid and unknown first (pass order), the half-alpha one last");
    }

    #[test]
    fn two_ds_models_on_one_ps2_scene_draw_it_once() {
        // replacements 0 and 2 are made of the same PS2 model, 1 of another
        let source = [7u64, 9, 7];
        let mut at = vec![(0usize, 'a'), (1, 'b'), (2, 'c')];
        super::ult_once(&mut at, |c| source[c.0]);
        assert_eq!(at, [(0, 'a'), (1, 'b')], "the first of each source, in the frame's order");
        let mut none: Vec<(usize, char)> = Vec::new();
        super::ult_once(&mut none, |c| source[c.0]);
        assert!(none.is_empty());
    }
}

#[cfg(all(test, feature = "gpu"))]
mod gpu_fallback_tests {
    use super::super::render3d::{self, GxPoly, GxVertex, Job};
    use super::*;

    /// A job with one (degenerate, one-vertex) polygon: not an empty frame, nothing drawn by it.
    fn one_polygon() -> Job {
        let mut job = render3d::tests::empty_job();
        job.verts.push(GxVertex { x: 0, y: 0, z: 0, w: 0x1000, sx: 0.0, sy: 0.0, ix: 0, iy: 0, r: 0.0, g: 0.0, b: 0.0, s: 0.0, t: 0.0 });
        job.polys.push(GxPoly { first: 0, n: 1, attr: 31 << 16, tex: 0, pal: 0, translucent: false, sortkey: 0 });
        job
    }

    #[test]
    fn a_failing_gpu_engine_leaves_the_software_picture_and_a_note() {
        let job = one_polygon();
        let want = render3d::render(&job, 1);
        let (got, failed) = gpu_or_software(&job, 1, || Err("no GPU adapter".to_string()));
        assert_eq!(failed.as_deref(), Some("no GPU adapter"));
        assert_eq!(got, want);
        let (got, failed) = gpu_or_software(&job, 1, || Ok((vec![7; 4], 0)));
        assert!(failed.is_none());
        assert_eq!(got.0, vec![7; 4], "a working engine's picture is used as it is");
    }

    #[test]
    fn a_frame_with_no_polygons_is_the_software_engines_and_never_asks_the_gpu() {
        let job = render3d::tests::empty_job();
        let want = render3d::render(&job, 1);
        let (got, failed) = gpu_or_software(&job, 1, || panic!("the GPU draws a frame with no polygons"));
        assert_eq!((got, failed), (want, None));
    }
}

impl Nds {
    /// The key of a replacement model's picture, by the PLTT_BASE its polygons carry (`ULT_PAL`).
    pub(super) fn ult_key(&self, tex: u32, pal: u32) -> Option<super::render3d::TexKey> {
        self.gx.ult.iter().flat_map(|u| &u.parts).find(|p| p.pal == pal && p.tex == tex).and_then(|p| p.key)
    }

    /// The frame being drawn: each run of batches' model and mesh (`None`: not out of a model file) and its polygons.
    pub(crate) fn frame_models(&mut self) -> Vec<(Option<super::modelsrc::ModelAt>, u32, u8)> {
        let srcs = self.gx.render.srcs.clone();
        let at = self.models_at(&srcs.iter().map(|s| s.0).collect::<Vec<_>>());
        at.into_iter().zip(srcs).map(|(m, s)| (m, s.1, s.2)).collect()
    }
}
