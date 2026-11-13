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
        objects.push(Object { at, packet_data: u32_at(d, at + 4)?, prim_data: u32_at(d, at + 0x80)?, joints });
    }
    Ok(Gig { scene_scale, objects })
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
