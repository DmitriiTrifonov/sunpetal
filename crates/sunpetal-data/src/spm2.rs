//! `SPM2`, the second model format of the `MODL` part (`docs/ULTIMATE-IMPORT.md`, section 7): a PS2
//! model whole — several materials, the vertices' colours, the bones — and what it stands for in the
//! DS game, named by the DS model (not by a texture): the game sends a model's polygons straight out of its
//! file in memory, and the file names the model (`nds/modelsrc.rs` of the runtime). Little-endian.
//!
//! ```text
//! model:    "SPM2", u8 flags (bit 0: bound to a DS model; bit 1: it stands for the anchor mesh only, not the whole DS model), f32 unit (positions × unit = the PS2 scene's units),
//!           [bound: u16 length + the DS model's name in its file's dictionary ("no46_room_402r"),
//!                   u32 the DS file's size (0: any; a few names are shared by several files), u16 the anchor: the DS mesh whose space the model is put in (0xFFFF: the mesh with most polygons in the frame), 12 × f32: rows x, y, z of the 3 × 3, then the shift
//!                   (a point of this model, after its bone and × unit, into the DS model's space)],
//!           u16 bones,     each: i16 parent (-1: a root), i16 standard joint (-1: none), i16 DS bone (-1: none),
//!                                6 × f32 rest: place x y z, turn x y z in radians (the bone = T · Ry · Rz · Rx, under its parent)
//!           u16 materials, each: u8 has picture, [u64 picture hash, u64 palette hash, u16 w, u16 h],
//!                                u8 pass (0-2, drawn in this order), u8 blend (0 solid, 1 cut-out, 2 see-through, 3 added, 255 not known),
//!                                u8 flags (bit 0 both sides, 1 clamp s, 2 clamp t)
//!           u32 vertices,  each 40 bytes: f32 place xyz (in its bone's space), f32 normal xyz, u8 rgba, f32 uv (0..1, v down as on the PS2),
//!                                u16 bone, u16 0
//!           u32 parts,     each: u16 material, u32 first index, u32 indices
//!           u32 indices,   each u32 (triangles; the parts cover them in order, with no gap)
//! ```
//! A picture is looked up in the `TEXS` part by its hashes (the first 8 bytes of the SHA-1 of the PS2 picture's
//! bytes and of its palette's): PS2 pictures have their own keys there, apart from the DS ones.

pub const MAGIC: &[u8; 4] = b"SPM2";

#[derive(Clone, Debug, PartialEq)]
pub struct Bind {
    /// The DS model this stands for: its name in the dictionary of its file.
    pub model: String,
    /// That file's size, where the name alone is shared by several files.
    pub size: Option<u32>,
    /// The anchor: the DS mesh whose space `place` leads into (`None`: the one with most polygons in the frame).
    pub mesh: Option<u16>,
    /// Only the anchor mesh is replaced (else every mesh of the DS model is).
    pub only_mesh: bool,
    /// Rows x, y, z of the 3 × 3, then the shift.
    pub place: [f32; 12],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bone {
    pub parent: Option<u16>,
    /// The PS2 engine's standard joint number (what its motions are keyed by).
    pub standard: Option<u16>,
    /// The DS model's bone that moves this one, where the DS game's own pose is used.
    pub ds: Option<u16>,
    /// Place x y z, turn x y z.
    pub rest: [f32; 6],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Picture {
    pub data: u64,
    pub pal: u64,
    pub w: u16,
    pub h: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Material {
    pub picture: Option<Picture>,
    pub pass: u8,
    pub blend: u8,
    pub flags: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub nrm: [f32; 3],
    pub rgba: [u8; 4],
    pub uv: [f32; 2],
    pub bone: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Part {
    pub material: u16,
    pub first: u32,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub name: String,
    pub unit: f32,
    pub bind: Option<Bind>,
    pub bones: Vec<Bone>,
    pub materials: Vec<Material>,
    pub verts: Vec<Vertex>,
    pub parts: Vec<Part>,
    pub idx: Vec<u32>,
}

fn opt(v: Option<u16>) -> [u8; 2] {
    v.map_or(-1i16, |x| x as i16).to_le_bytes()
}

impl Model {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = MAGIC.to_vec();
        o.push(self.bind.as_ref().map_or(0, |b| 1 | (b.only_mesh as u8) << 1));
        o.extend_from_slice(&self.unit.to_le_bytes());
        if let Some(b) = &self.bind {
            o.extend_from_slice(&(b.model.len() as u16).to_le_bytes());
            o.extend_from_slice(b.model.as_bytes());
            o.extend_from_slice(&b.size.unwrap_or(0).to_le_bytes());
            o.extend_from_slice(&b.mesh.unwrap_or(0xFFFF).to_le_bytes());
            b.place.iter().for_each(|f| o.extend_from_slice(&f.to_le_bytes()));
        }
        o.extend_from_slice(&(self.bones.len() as u16).to_le_bytes());
        for b in &self.bones {
            o.extend_from_slice(&opt(b.parent));
            o.extend_from_slice(&opt(b.standard));
            o.extend_from_slice(&opt(b.ds));
            b.rest.iter().for_each(|f| o.extend_from_slice(&f.to_le_bytes()));
        }
        o.extend_from_slice(&(self.materials.len() as u16).to_le_bytes());
        for m in &self.materials {
            o.push(m.picture.is_some() as u8);
            if let Some(p) = m.picture {
                o.extend_from_slice(&p.data.to_le_bytes());
                o.extend_from_slice(&p.pal.to_le_bytes());
                o.extend_from_slice(&p.w.to_le_bytes());
                o.extend_from_slice(&p.h.to_le_bytes());
            }
            o.extend_from_slice(&[m.pass, m.blend, m.flags]);
        }
        o.extend_from_slice(&(self.verts.len() as u32).to_le_bytes());
        for v in &self.verts {
            v.pos.iter().chain(&v.nrm).for_each(|f| o.extend_from_slice(&f.to_le_bytes()));
            o.extend_from_slice(&v.rgba);
            v.uv.iter().for_each(|f| o.extend_from_slice(&f.to_le_bytes()));
            o.extend_from_slice(&v.bone.to_le_bytes());
            o.extend_from_slice(&[0, 0]);
        }
        o.extend_from_slice(&(self.parts.len() as u32).to_le_bytes());
        for p in &self.parts {
            o.extend_from_slice(&p.material.to_le_bytes());
            o.extend_from_slice(&p.first.to_le_bytes());
            o.extend_from_slice(&p.count.to_le_bytes());
        }
        o.extend_from_slice(&(self.idx.len() as u32).to_le_bytes());
        self.idx.iter().for_each(|i| o.extend_from_slice(&i.to_le_bytes()));
        o
    }

    /// A model from its bytes. Refuses: a cut-short one or one with bytes after it; a number that is not
    /// finite; a bone whose parent is not a bone, or a loop of parents; a vertex of no bone; a part of no
    /// material, not whole triangles, or parts that do not cover the indices in order; an index past the vertices.
    pub fn decode(name: &str, d: &[u8]) -> Result<Model, String> {
        let bad = |why: &str| format!("model {}: {}", name, why);
        let mut at = 0usize;
        let mut take = |n: usize| -> Result<&[u8], String> {
            let s = d.get(at..at.checked_add(n).ok_or_else(|| bad("too long"))?).ok_or_else(|| bad("cut short"))?;
            at += n;
            Ok(s)
        };
        let u16_at = |s: &[u8], i: usize| u16::from_le_bytes([s[i], s[i + 1]]);
        let u32_at = |s: &[u8], i: usize| u32::from_le_bytes(s[i..i + 4].try_into().unwrap());
        let u64_at = |s: &[u8], i: usize| u64::from_le_bytes(s[i..i + 8].try_into().unwrap());
        let f32_at = |s: &[u8], i: usize| -> Result<f32, String> {
            let f = f32::from_le_bytes(s[i..i + 4].try_into().unwrap());
            if f.is_finite() {
                Ok(f)
            } else {
                Err(bad("a number is not finite"))
            }
        };
        let some = |v: u16| (v != 0xFFFF).then_some(v);
        if take(4)? != MAGIC {
            return Err(bad("not a model"));
        }
        let flags = take(1)?[0];
        if flags > 3 || flags == 2 {
            return Err(bad("unknown flags"));
        }
        let unit = f32_at(take(4)?, 0)?;
        let bind = if flags & 1 != 0 {
            let n = u16_at(take(2)?, 0) as usize;
            let model = String::from_utf8(take(n)?.to_vec()).map_err(|_| bad("the DS model's name is not text"))?;
            let size = Some(u32_at(take(4)?, 0)).filter(|&x| x != 0);
            let mesh = some(u16_at(take(2)?, 0));
            let p = take(48)?;
            let mut place = [0.0; 12];
            for (i, x) in place.iter_mut().enumerate() {
                *x = f32_at(p, 4 * i)?;
            }
            Some(Bind { model, size, mesh, only_mesh: flags & 2 != 0, place })
        } else {
            None
        };
        let nb = u16_at(take(2)?, 0) as usize;
        let mut bones = Vec::with_capacity(nb);
        for _ in 0..nb {
            let b = take(30)?;
            let mut rest = [0.0; 6];
            for (i, x) in rest.iter_mut().enumerate() {
                *x = f32_at(b, 6 + 4 * i)?;
            }
            bones.push(Bone { parent: some(u16_at(b, 0)), standard: some(u16_at(b, 2)), ds: some(u16_at(b, 4)), rest });
        }
        for start in 0..nb {
            let (mut at, mut steps) = (start, 0);
            while let Some(p) = bones[at].parent {
                at = p as usize;
                steps += 1;
                if at >= nb || steps > nb {
                    return Err(bad("a bone's parent is not a bone, or the parents loop"));
                }
            }
        }
        let nm = u16_at(take(2)?, 0) as usize;
        let mut materials = Vec::with_capacity(nm);
        for _ in 0..nm {
            let picture = match take(1)?[0] {
                0 => None,
                1 => {
                    let p = take(20)?;
                    Some(Picture { data: u64_at(p, 0), pal: u64_at(p, 8), w: u16_at(p, 16), h: u16_at(p, 18) })
                }
                _ => return Err(bad("bad picture flag")),
            };
            let m = take(3)?;
            if m[0] > 2 {
                return Err(bad("a pass past 2"));
            }
            materials.push(Material { picture, pass: m[0], blend: m[1], flags: m[2] });
        }
        let nv = u32_at(take(4)?, 0) as usize;
        let vb = take(nv.checked_mul(40).ok_or_else(|| bad("too many vertices"))?)?;
        let mut verts = Vec::with_capacity(nv);
        for v in vb.chunks_exact(40) {
            let bone = u16_at(v, 36);
            if bone as usize >= nb.max(1) {
                return Err(bad("a vertex of no bone"));
            }
            verts.push(Vertex {
                pos: [f32_at(v, 0)?, f32_at(v, 4)?, f32_at(v, 8)?],
                nrm: [f32_at(v, 12)?, f32_at(v, 16)?, f32_at(v, 20)?],
                rgba: [v[24], v[25], v[26], v[27]],
                uv: [f32_at(v, 28)?, f32_at(v, 32)?],
                bone,
            });
        }
        let np = u32_at(take(4)?, 0) as usize;
        let pb = take(np.checked_mul(10).ok_or_else(|| bad("too many parts"))?)?;
        let parts: Vec<Part> = pb.chunks_exact(10).map(|p| Part { material: u16_at(p, 0), first: u32_at(p, 2), count: u32_at(p, 6) }).collect();
        let ni = u32_at(take(4)?, 0) as usize;
        let ib = take(ni.checked_mul(4).ok_or_else(|| bad("too many indices"))?)?;
        let idx: Vec<u32> = ib.chunks_exact(4).map(|x| u32_at(x, 0)).collect();
        if idx.iter().any(|&i| i as usize >= nv) {
            return Err(bad("an index past the vertices"));
        }
        let mut next = 0u64;
        for p in &parts {
            if p.material as usize >= nm || p.count % 3 != 0 || p.first as u64 != next {
                return Err(bad("a part of no material, not whole triangles, or out of order"));
            }
            next += p.count as u64;
        }
        if next != ni as u64 {
            return Err(bad("the parts do not cover the indices"));
        }
        if at != d.len() {
            return Err(bad("bytes after the model"));
        }
        Ok(Model { name: name.to_owned(), unit, bind, bones, materials, verts, parts, idx })
    }
}

/// The `SPM2` models of a `MODL` section (the older `SPM1` ones are left to `models::unpack`).
pub fn unpack(d: &[u8]) -> Result<Vec<Model>, String> {
    crate::models::entries(d)?.into_iter().filter(|(_, b)| b.starts_with(MAGIC)).map(|(n, b)| Model::decode(&n, b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Model {
        let v = |x: f32, bone: u16| Vertex { pos: [x, 2.0, 3.0], nrm: [0.0, 1.0, 0.0], rgba: [127, 127, 127, 128], uv: [0.25, 0.75], bone };
        Model {
            name: "room".into(),
            unit: 0.1,
            bind: Some(Bind {
                model: "no46_room_402r".into(),
                size: Some(80376),
                mesh: None,
                only_mesh: false,
                place: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.5, 0.0, -2.0],
            }),
            bones: vec![
                Bone { parent: None, standard: Some(0), ds: Some(0), rest: [0.0, 9.0, 0.0, 0.0, 0.0, 0.0] },
                Bone { parent: Some(0), standard: Some(1), ds: None, rest: [0.0, 1.5, 0.0, 0.0, -0.03, 0.0] },
            ],
            materials: vec![
                Material { picture: Some(Picture { data: 0xaa, pal: 0xbb, w: 128, h: 128 }), pass: 0, blend: 0, flags: 0 },
                Material { picture: None, pass: 1, blend: 2, flags: 1 },
            ],
            verts: vec![v(0.0, 0), v(1.0, 0), v(2.0, 1)],
            parts: vec![Part { material: 0, first: 0, count: 3 }, Part { material: 1, first: 3, count: 3 }],
            idx: vec![0, 1, 2, 2, 1, 0],
        }
    }

    #[test]
    fn a_model_round_trips() {
        let m = model();
        assert_eq!(Model::decode("room", &m.encode()).unwrap(), m);
        let mut free = model();
        (free.bind, free.bones) = (None, vec![]);
        free.verts.iter_mut().for_each(|v| v.bone = 0);
        assert_eq!(Model::decode("room", &free.encode()).unwrap(), free, "no binding and no bones: one still model");
    }

    #[test]
    fn a_bad_model_is_refused() {
        let good = model().encode();
        assert!(Model::decode("x", &good[..good.len() - 1]).is_err(), "cut short");
        assert!(Model::decode("x", &[good.clone(), vec![0]].concat()).is_err(), "bytes after");
        assert!(Model::decode("x", b"SPM1").is_err());
        let check = |why: &str, f: &dyn Fn(&mut Model)| {
            let mut m = model();
            f(&mut m);
            assert!(Model::decode("x", &m.encode()).is_err(), "{}", why);
        };
        check("index past the vertices", &|m| m.idx[1] = 9);
        check("NaN", &|m| m.verts[0].pos[0] = f32::NAN);
        check("infinite rest", &|m| m.bones[1].rest[0] = f32::INFINITY);
        check("parents loop", &|m| m.bones[0].parent = Some(1));
        check("parent is not a bone", &|m| m.bones[1].parent = Some(7));
        check("vertex of no bone", &|m| m.verts[2].bone = 2);
        check("part of no material", &|m| m.parts[1].material = 2);
        check("not whole triangles", &|m| (m.parts[0].count, m.parts[1].first, m.parts[1].count) = (2, 2, 4));
        check("parts with a gap", &|m| m.parts[1].first = 4);
        check("parts short of the indices", &|m| m.parts.truncate(1));
        check("pass past 2", &|m| m.materials[0].pass = 3);
        let mut one = model();
        one.bind.as_mut().unwrap().only_mesh = true;
        assert_eq!(Model::decode("room", &one.encode()).unwrap(), one, "the anchor-only flag");
        let mut flags = model().encode();
        flags[4] = 2;
        assert!(Model::decode("x", &flags).is_err(), "anchor-only with no binding");
    }

    #[test]
    fn a_section_holds_both_formats() {
        let old =
            crate::models::Model { name: "car".into(), key: None, verts: vec![crate::models::Vertex { pos: [0.0; 3], nrm: [0.0, 1.0, 0.0], uv: [0.0; 2] }; 3], idx: vec![0, 1, 2] };
        let d = crate::models::pack(&[("car", old.encode()), ("room", model().encode())]);
        assert_eq!(unpack(&d).unwrap(), vec![model()]);
        assert_eq!(crate::models::unpack(&d).unwrap(), vec![old]);
    }
}
