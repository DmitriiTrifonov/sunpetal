//! The 3D models part of a data file (`MODL`, K-110, `docs/ULTIMATE.md`): meshes read from the PS2 version,
//! which the 3D renderer may draw in place of a DS object (K-111).
//!
//! ```text
//! section: "MDL1", u32 count, per model: u16 name length, name, u32 length, the model
//! model:   "SPM1", u8 has_key, [DS key: u64 data hash, u64 palette hash, u8 format, u16 w, u16 h],
//!          u32 vertex count, u32 index count,
//!          per vertex 8 × f32: position xyz (in the DS object's own units: the manifest's orientation, scale and shift are baked in), normal xyz, texture uv (0..1, v up),
//!          per index u32 (triangles, three to each)
//! ```
//! A model's texture is not in it: pictures go through the `TEXS` part (by the key of the DS texture the
//! model replaces, `has_key`). Models are made by `tools/ps2/mkmodels.py` and put in by `sunpetal-pack-models`.
use std::path::Path;

/// The section's tag in a data file.
pub const TAG: &[u8; 4] = b"MODL";
const MAGIC: &[u8; 4] = b"MDL1";
const MODEL: &[u8; 4] = b"SPM1";

/// A DS texture's key (as `texpack.rs` makes it): the object the model stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DsKey {
    pub data: u64,
    pub pal: u64,
    pub fmt: u8,
    pub w: u16,
    pub h: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub nrm: [f32; 3],
    pub uv: [f32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub name: String,
    pub key: Option<DsKey>,
    pub verts: Vec<Vertex>,
    /// Triangles, three indices each.
    pub idx: Vec<u32>,
}

impl Model {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = MODEL.to_vec();
        match &self.key {
            Some(k) => {
                o.push(1);
                o.extend_from_slice(&k.data.to_le_bytes());
                o.extend_from_slice(&k.pal.to_le_bytes());
                o.push(k.fmt);
                o.extend_from_slice(&k.w.to_le_bytes());
                o.extend_from_slice(&k.h.to_le_bytes());
            }
            None => o.push(0),
        }
        o.extend_from_slice(&(self.verts.len() as u32).to_le_bytes());
        o.extend_from_slice(&(self.idx.len() as u32).to_le_bytes());
        for v in &self.verts {
            for f in v.pos.iter().chain(&v.nrm).chain(&v.uv) {
                o.extend_from_slice(&f.to_le_bytes());
            }
        }
        for i in &self.idx {
            o.extend_from_slice(&i.to_le_bytes());
        }
        o
    }

    /// A model from its bytes (`name` is the section entry's); refuses a cut-short one, an index past the
    /// vertices, a count that is not whole triangles and a vertex that is not a finite number.
    pub fn decode(name: &str, d: &[u8]) -> Result<Model, String> {
        let bad = |why: &str| format!("model {}: {}", name, why);
        let mut at = 0usize;
        let mut take = |n: usize| -> Result<&[u8], String> {
            let s = d.get(at..at.checked_add(n).ok_or_else(|| bad("too long"))?).ok_or_else(|| bad("cut short"))?;
            at += n;
            Ok(s)
        };
        if take(4)? != MODEL {
            return Err(bad("not a model"));
        }
        let key = match take(1)?[0] {
            0 => None,
            1 => {
                let k = take(21)?;
                Some(DsKey {
                    data: u64::from_le_bytes(k[0..8].try_into().unwrap()),
                    pal: u64::from_le_bytes(k[8..16].try_into().unwrap()),
                    fmt: k[16],
                    w: u16::from_le_bytes([k[17], k[18]]),
                    h: u16::from_le_bytes([k[19], k[20]]),
                })
            }
            _ => return Err(bad("bad key flag")),
        };
        let c = take(8)?;
        let (nv, ni) = (u32::from_le_bytes(c[0..4].try_into().unwrap()) as usize, u32::from_le_bytes(c[4..8].try_into().unwrap()) as usize);
        if ni % 3 != 0 {
            return Err(bad("the indices are not whole triangles"));
        }
        let vb = take(nv.checked_mul(32).ok_or_else(|| bad("too many vertices"))?)?;
        let mut verts = Vec::with_capacity(nv);
        for v in vb.chunks_exact(32) {
            let f: Vec<f32> = v.chunks_exact(4).map(|x| f32::from_le_bytes(x.try_into().unwrap())).collect();
            if f.iter().any(|x| !x.is_finite()) {
                return Err(bad("a vertex is not a finite number"));
            }
            verts.push(Vertex { pos: [f[0], f[1], f[2]], nrm: [f[3], f[4], f[5]], uv: [f[6], f[7]] });
        }
        let ib = take(ni.checked_mul(4).ok_or_else(|| bad("too many indices"))?)?;
        let idx: Vec<u32> = ib.chunks_exact(4).map(|x| u32::from_le_bytes(x.try_into().unwrap())).collect();
        if idx.iter().any(|&i| i as usize >= nv) {
            return Err(bad("an index past the vertices"));
        }
        if at != d.len() {
            return Err(bad("bytes after the model"));
        }
        Ok(Model { name: name.to_owned(), key, verts, idx })
    }
}

/// The `*.spm` files of a folder as a section, in the order of their names (the name is the file's).
pub fn pack_dir(dir: &Path) -> Result<Vec<u8>, String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {}", dir.display(), e))?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".spm"))
        .collect();
    names.sort();
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&(names.len() as u32).to_le_bytes());
    for n in &names {
        let stem = &n[..n.len() - 4];
        let data = std::fs::read(dir.join(n)).map_err(|e| format!("{}: {}", n, e))?;
        Model::decode(stem, &data)?;
        out.extend_from_slice(&(stem.len() as u16).to_le_bytes());
        out.extend_from_slice(stem.as_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
    }
    Ok(out)
}

/// The models of a section.
pub fn unpack(d: &[u8]) -> Result<Vec<Model>, String> {
    let bad = || "models part: cut short or not one".to_string();
    if d.len() < 8 || &d[..4] != MAGIC {
        return Err(bad());
    }
    let n = u32::from_le_bytes(d[4..8].try_into().unwrap()) as usize;
    let (mut at, mut out) = (8, Vec::new());
    for _ in 0..n {
        let nl = u16::from_le_bytes(d.get(at..at + 2).ok_or_else(bad)?.try_into().unwrap()) as usize;
        let name = String::from_utf8_lossy(d.get(at + 2..at + 2 + nl).ok_or_else(bad)?).into_owned();
        at += 2 + nl;
        let len = u32::from_le_bytes(d.get(at..at + 4).ok_or_else(bad)?.try_into().unwrap()) as usize;
        out.push(Model::decode(&name, d.get(at + 4..at + 4 + len).ok_or_else(bad)?)?);
        at += 4 + len;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(name: &str, key: bool) -> Model {
        let v = |x: f32| Vertex { pos: [x, 2.0, 3.0], nrm: [0.0, 1.0, 0.0], uv: [0.25, 0.75] };
        Model { name: name.into(), key: key.then_some(DsKey { data: 0xaa, pal: 0xbb, fmt: 5, w: 128, h: 128 }), verts: vec![v(0.0), v(1.0), v(2.0)], idx: vec![0, 1, 2, 2, 1, 0] }
    }

    #[test]
    fn a_model_round_trips_and_a_bad_one_is_refused() {
        for k in [true, false] {
            let m = model("car", k);
            assert_eq!(Model::decode("car", &m.encode()).unwrap(), m);
        }
        let good = model("car", true).encode();
        assert!(Model::decode("x", &good[..good.len() - 1]).is_err(), "cut short");
        assert!(Model::decode("x", &[good.clone(), vec![0]].concat()).is_err(), "bytes after");
        let mut m = model("car", false);
        m.idx[1] = 9;
        assert!(Model::decode("x", &m.encode()).is_err(), "index past the vertices");
        m.idx = vec![0, 1];
        assert!(Model::decode("x", &m.encode()).is_err(), "not whole triangles");
        m = model("car", false);
        m.verts[0].pos[0] = f32::NAN;
        assert!(Model::decode("x", &m.encode()).is_err(), "NaN");
        assert!(Model::decode("x", b"nope").is_err());
    }

    #[test]
    fn a_folder_goes_into_a_section_and_comes_back() {
        let dir = std::env::temp_dir().join(format!("sunpetal-spm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.spm"), model("b", true).encode()).unwrap();
        std::fs::write(dir.join("a.spm"), model("a", false).encode()).unwrap();
        std::fs::write(dir.join("notes.txt"), b"left out").unwrap();
        let d = pack_dir(&dir).unwrap();
        let back = unpack(&d).unwrap();
        assert_eq!(back, vec![model("a", false), model("b", true)]);
        assert!(unpack(&d[..d.len() - 1]).is_err());
        assert!(unpack(b"nope").is_err());
        std::fs::write(dir.join("c.spm"), b"junk").unwrap();
        assert!(pack_dir(&dir).is_err(), "a bad model stops the packing");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
