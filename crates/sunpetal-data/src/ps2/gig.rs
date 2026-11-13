//! `G101` models (У1.6): the header, the objects and their joints. Layout from the game's debug types
//! (`GIG_HEADER`, `GIG_OBJHEADER`, `GIG_JOINTHEADER`); geometry is the next steps (У1.7, У1.8).
//! Offsets inside a model are from its start.

pub const MAGIC: &[u8; 4] = b"G101";
const OBJECT_TABLE: usize = 0x50;
const JOINTS: usize = 0xA0;
const JOINT_SIZE: usize = 0x140;

#[derive(Clone, Debug, PartialEq)]
pub struct Joint {
    pub is_bone_ref: bool,
    pub key_no: u16,
    /// Index of the first child joint within the object, -1 for none.
    pub child: i16,
    /// Index of the next sibling, -1 for none.
    pub next: i16,
    /// Joint of the standard skeleton a motion (`GIM`) addresses (`STPTable[StandardJointNo]`).
    pub standard_joint: i16,
    /// `InitKey`: position and rotation (radians) of the rest pose.
    pub init_key: [f32; 6],
}

#[derive(Clone, Debug)]
pub struct Object {
    /// Offset of the object header from the start of the model.
    pub at: usize,
    pub packet_data: u32,
    /// Offset of the first `PrimData` section.
    pub prim_data: u32,
    pub joints: Vec<Joint>,
    pub series: Vec<Series>,
}

/// One vertex of a series: position, normal, colour (0..255 floats as stored), ST and the matrix slot (a bone of the series).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub nrm: [f32; 3],
    pub rgba: [f32; 4],
    pub st: [f32; 2],
    pub slot: u32,
}

/// A strip or a list of vertices sharing a material (`TEX0`, GS `PRIM`).
#[derive(Clone, Debug)]
pub struct Series {
    /// `PrimData` section: 0 `getphPrim`, 1 `getphPrimT`, 2 `getphPrimT2`.
    pub pass: u32,
    pub tex0: u64,
    /// GS `PRIM` (bit 4 is TME).
    pub prim: u32,
    pub verts: Vec<Vertex>,
    /// Joints of the packet's matrix tags (1: rigid, 2-3: blended); a vertex's `slot` indexes them.
    pub bones: Vec<u16>,
}

impl Series {
    /// GS primitive type: 3 triangle list, 4 triangle strip, 5 fan.
    pub fn kind(&self) -> u32 {
        self.prim & 7
    }

    pub fn triangles(&self) -> usize {
        let n = self.verts.len();
        match self.kind() {
            4 => n.saturating_sub(2),
            3 => n / 3,
            _ => 0,
        }
    }
}

impl Vertex {
    /// The export's 40 bytes: pos, normal (f32), rgba (u8, rounded), st (f32), slot, 3 zero bytes.
    pub fn export_bytes(&self) -> [u8; 40] {
        let mut o = [0u8; 40];
        for (i, v) in self.pos.iter().chain(&self.nrm).enumerate() {
            o[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        for (i, c) in self.rgba.iter().enumerate() {
            o[24 + i] = c.clamp(0.0, 255.0).round_ties_even() as u8;
        }
        o[28..32].copy_from_slice(&self.st[0].to_le_bytes());
        o[32..36].copy_from_slice(&self.st[1].to_le_bytes());
        o[36] = self.slot as u8;
        o
    }
}

#[derive(Clone, Debug)]
pub struct Gig {
    pub scene_scale: f32,
    pub objects: Vec<Object>,
}

fn u32_at(d: &[u8], at: usize) -> Result<u32, String> {
    d.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).ok_or_else(|| format!("read past the end at {at:#x}"))
}

fn i16_at(d: &[u8], at: usize) -> Result<i16, String> {
    d.get(at..at + 2).map(|b| i16::from_le_bytes(b.try_into().unwrap())).ok_or_else(|| format!("read past the end at {at:#x}"))
}

fn f32_at(d: &[u8], at: usize) -> Result<f32, String> {
    u32_at(d, at).map(f32::from_bits)
}

/// Size of the model in bytes: the body size at `+4` plus the 0x20 header the game counts apart (the export hashes this span).
pub fn size(d: &[u8]) -> Result<usize, String> {
    Ok(u32_at(d, 4)? as usize + 0x20)
}

/// Reads the header, the object table and every object's joints from a slice that starts at the `G101` magic.
pub fn parse(d: &[u8]) -> Result<Gig, String> {
    if d.get(..4) != Some(MAGIC) {
        return Err("not a G101 model".into());
    }
    let n = i16_at(d, 0xC)?;
    if !(0..512).contains(&n) {
        return Err(format!("object count {n}"));
    }
    let scene_scale = f32_at(d, 0x20)?;
    let mut objects = Vec::with_capacity(n as usize);
    for i in 0..n as usize {
        let at = u32_at(d, OBJECT_TABLE + 4 * i)? as usize;
        let nj = i16_at(d, at + 8)?;
        if !(0..=1024).contains(&nj) {
            return Err(format!("object {i}: joint count {nj}"));
        }
        let mut joints = Vec::with_capacity(nj as usize);
        for j in 0..nj as usize {
            let q = at + JOINTS + JOINT_SIZE * j;
            let mut init_key = [0f32; 6];
            for (k, v) in init_key.iter_mut().enumerate() {
                *v = f32_at(d, q + 0x10 + 4 * k)?;
            }
            joints.push(Joint {
                is_bone_ref: i16_at(d, q)? != 0,
                key_no: i16_at(d, q + 2)? as u16,
                child: i16_at(d, q + 4)?,
                next: i16_at(d, q + 6)?,
                standard_joint: i16_at(d, q + 0xC)?,
                init_key,
            });
        }
        objects.push(Object { at, packet_data: u32_at(d, at + 4)?, prim_data: u32_at(d, at + 0x80)?, joints, series: series(d, u32_at(d, at + 0x80)? as usize) });
    }
    Ok(Gig { scene_scale, objects })
}

const UNPACK_BYTES: [usize; 16] = [4, 2, 1, 0, 8, 4, 2, 0, 12, 6, 3, 0, 16, 8, 4, 2];

struct Tag {
    at: usize,
    qwc: usize,
    id: u32,
    v: [u32; 2],
}

/// The DMA chain from `start`: `cnt` (1) data follows, `next` (2) jumps to `ADDR`, `ref` (3) has its data outside, `end` (7) stops.
fn tags(d: &[u8], start: usize) -> Vec<Tag> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut p = start;
    for _ in 0..4096 {
        if !seen.insert(p) || p + 16 > d.len() {
            break;
        }
        let w = |k: usize| u32::from_le_bytes(d[p + k..p + k + 4].try_into().unwrap());
        let (w0, addr) = (w(0), w(4));
        let (qwc, id) = ((w0 & 0xFFFF) as usize, (w0 >> 28) & 7);
        out.push(Tag { at: p, qwc, id, v: [w(8), w(12)] });
        match id {
            7 => break,
            2 => p = addr as usize,
            1 => p += 16 + qwc * 16,
            3 => p += 16,
            _ => break,
        }
    }
    out
}

/// `UNPACK V4-32` blocks of a tag's VIF stream: (VU address, quadword count, data offset). A `ref` tag's data is outside the file.
fn unpacks(d: &[u8], t: &Tag) -> Result<Vec<(usize, usize)>, String> {
    let mut out = Vec::new();
    let mut pos = t.at + 16;
    let end = if t.id != 3 { pos + t.qwc * 16 } else { pos };
    let mut i = 0;
    loop {
        let c = if i < 2 {
            i += 1;
            t.v[i - 1]
        } else if pos + 4 <= end {
            pos += 4;
            u32::from_le_bytes(d.get(pos - 4..pos).ok_or("tag past the end")?.try_into().unwrap())
        } else {
            break;
        };
        let (cmd, num) = ((c >> 24) & 0x7F, (c >> 16) & 0xFF);
        if cmd >= 0x60 {
            let n = if num == 0 { 256 } else { num as usize };
            if t.id != 3 {
                let b = UNPACK_BYTES[(cmd & 0xF) as usize];
                if b == 0 {
                    return Err("reserved UNPACK format".into());
                }
                if cmd & 0xF == 0xC {
                    out.push((n, pos));
                }
                pos += (n * b + 3) & !3;
            }
        } else if (cmd == 0x30 || cmd == 0x31) && t.id != 3 {
            pos += 16;
        } else if cmd == 0x4A || cmd == 0x4B {
            pos += (if num == 0 { 256 } else { num as usize }) * 8;
        } else if cmd > 0x20 && i > 2 {
            break;
        }
    }
    Ok(out)
}

/// The series of one UNPACK block of `n` quadwords at `o`; `None` when a series runs out of the block or the file.
fn block(d: &[u8], o: usize, n: usize) -> Option<Vec<(u64, u32, Vec<Vertex>)>> {
    let u32_ = |at: usize| d.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let f = |at: usize| u32_(at).map(f32::from_bits);
    let mut out = Vec::new();
    let mut q = 0;
    while q + 4 <= n {
        let h = o + q * 16;
        let nv = (u32_(h + 0x30)? & 0x7FFF) as usize;
        let prim = (u32_(h + 0x34)? >> 15) & 0x7FF;
        let tex0 = u64::from(u32_(h + 0x20)?) | u64::from(u32_(h + 0x24)?) << 32;
        if q + 4 + nv * 4 > n {
            return None;
        }
        let mut verts = Vec::with_capacity(nv);
        for k in 0..nv {
            let v = h + 0x40 + 0x40 * k;
            let (mut u, mut w) = if prim & 0x10 != 0 { (f(v + 0x30)?, f(v + 0x34)?) } else { (0.0, 0.0) };
            if !(u.is_finite() && w.is_finite() && u.abs() < 1e6 && w.abs() < 1e6) {
                (u, w) = (0.0, 0.0);
            }
            verts.push(Vertex {
                pos: [f(v)?, f(v + 4)?, f(v + 8)?],
                nrm: [f(v + 0x10)?, f(v + 0x14)?, f(v + 0x18)?],
                rgba: [f(v + 0x20)?, f(v + 0x24)?, f(v + 0x28)?, f(v + 0x2C)?],
                st: [u, w],
                slot: u32_(v + 0x3C)?,
            });
        }
        out.push((tex0, prim, verts));
        q += 4 + nv * 4;
    }
    (q == n).then_some(out)
}

/// All series of an object: the `PrimData` sections (`[count, end offset, "PadObj"]` + a DMA chain) in file order.
fn series(d: &[u8], start: usize) -> Vec<Series> {
    let mut out = Vec::new();
    let (mut pos, mut pass) = (start, 0);
    while d.get(pos + 8..pos + 14) == Some(b"PadObj") {
        let end_off = u32::from_le_bytes(d[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let mut mats: Vec<(u16, usize)> = Vec::new();
        for t in tags(d, pos + 0x10) {
            if t.id == 3 {
                if mats.len() == 3 {
                    mats.clear();
                }
                mats.push((((u32::from_le_bytes(d[t.at..t.at + 4].try_into().unwrap()) >> 16) & 0x3FF) as u16, t.qwc));
            }
            let Ok(blocks) = unpacks(d, &t) else { continue };
            for (num, off) in blocks {
                if num < 4 {
                    continue;
                }
                if let Some(ss) = block(d, off, num) {
                    for (tex0, prim, verts) in ss {
                        let bones = mats.iter().filter(|m| m.1 != 0).map(|m| m.0).collect();
                        out.push(Series { pass, tex0, prim, verts, bones });
                    }
                }
            }
        }
        pass += 1;
        pos = end_off + 0x10;
    }
    out
}

impl Gig {
    pub fn joint_count(&self) -> usize {
        self.objects.iter().map(|o| o.joints.len()).sum()
    }
}

/// Offsets of the plausible `G101` headers in a file (aligned to 16, a sane object count and scale).
pub fn find(d: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 0x60 <= d.len() {
        if &d[at..at + 4] == MAGIC {
            let n = i16::from_le_bytes([d[at + 0xC], d[at + 0xD]]);
            let sc = f32::from_bits(u32::from_le_bytes(d[at + 0x20..at + 0x24].try_into().unwrap()));
            if (1..512).contains(&n) && sc.is_finite() && sc > 0.0 && sc < 100.0 {
                out.push(at);
            }
        }
        at += 16;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic() -> Vec<u8> {
        let mut d = vec![0u8; 0x400];
        d[..4].copy_from_slice(MAGIC);
        d[4..8].copy_from_slice(&0x3E0u32.to_le_bytes());
        d[0xC..0xE].copy_from_slice(&1i16.to_le_bytes());
        d[0x20..0x24].copy_from_slice(&0.1f32.to_le_bytes());
        d[0x50..0x54].copy_from_slice(&0x60u32.to_le_bytes());
        d[0x60 + 8..0x60 + 10].copy_from_slice(&2i16.to_le_bytes());
        d[0x60 + 0x80..0x60 + 0x84].copy_from_slice(&0x300u32.to_le_bytes());
        let j0 = 0x60 + JOINTS;
        d[j0 + 4..j0 + 6].copy_from_slice(&1i16.to_le_bytes());
        d[j0 + 6..j0 + 8].copy_from_slice(&(-1i16).to_le_bytes());
        d[j0 + 0x10..j0 + 0x14].copy_from_slice(&2.5f32.to_le_bytes());
        d
    }

    #[test]
    fn header_object_and_joints() {
        let d = synthetic();
        let g = parse(&d).unwrap();
        assert_eq!(g.scene_scale, 0.1);
        assert_eq!(g.objects.len(), 1);
        assert_eq!(g.joint_count(), 2);
        let j = &g.objects[0].joints[0];
        assert_eq!((j.child, j.next, j.init_key[0]), (1, -1, 2.5));
        assert_eq!(g.objects[0].prim_data, 0x300);
    }
}
