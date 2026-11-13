//! `GIM0100` motions (У1.9): skeletal animation keys per joint. Layout from the game's debug types
//! (`GIM_HEADER`, `GIM_PHEADER`, per-joint key lists). Offsets inside a motion are from its start.
//! Interpolation and playback are a consumer's job; this only reads what is stored.

pub const MAGIC: &[u8; 8] = b"GIM0100\0";

#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub frame: u16,
    pub flag: i16,
    /// Absent in a rotation-only joint (flag 1): there the key is 0x10 bytes, not 0x20.
    pub pos: Option<[f32; 3]>,
    /// Radians, applied as `Ry·Rz·Rx` after the translation.
    pub rot: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Joint {
    pub flag: i32,
    pub child: u8,
    pub next: u8,
    pub keys: Vec<Key>,
}

#[derive(Clone, Debug)]
pub struct Motion {
    pub bone_type: u16,
    /// `STPTable`: for every standard joint, the motion's joint that drives it.
    pub stp_table: Vec<u8>,
    pub flags: u16,
    /// Last frame (`AnimRange`).
    pub range: u32,
    /// `None`: no data for this joint (offset 0).
    pub joints: Vec<Option<Joint>>,
}

fn rd<const N: usize>(d: &[u8], at: usize) -> Result<[u8; N], String> {
    d.get(at..at + N).map(|b| b.try_into().unwrap()).ok_or_else(|| format!("read past the end at {at:#x}"))
}

fn u16_at(d: &[u8], at: usize) -> Result<u16, String> {
    rd(d, at).map(u16::from_le_bytes)
}

fn u32_at(d: &[u8], at: usize) -> Result<u32, String> {
    rd(d, at).map(u32::from_le_bytes)
}

fn f32_at(d: &[u8], at: usize) -> Result<f32, String> {
    u32_at(d, at).map(f32::from_bits)
}

/// Reads a motion from a slice that starts at the `GIM0100` magic.
pub fn parse(d: &[u8]) -> Result<Motion, String> {
    if d.get(..8) != Some(MAGIC) {
        return Err("not a GIM0100 motion".into());
    }
    let bone_type = u16_at(d, 8)?;
    let n_std = u16_at(d, 10)? as usize;
    let ph = u32_at(d, 12)? as usize;
    let stp_table = d.get(0x10..0x10 + n_std).ok_or("STPTable past the end")?.to_vec();
    let nj = u16_at(d, ph)? as usize;
    let flags = u16_at(d, ph + 2)?;
    let range = u32_at(d, ph + 4)?;
    let mut joints = Vec::with_capacity(nj);
    for j in 0..nj {
        let o = u32_at(d, ph + 0x10 + 4 * j)? as usize;
        if o == 0 {
            joints.push(None);
            continue;
        }
        let flag = i32::from_le_bytes(rd(d, o)?);
        let (child, next) = (d[o + 4], d[o + 5]);
        let nk = u16_at(d, o + 6)? as usize;
        let rot_only = flag == 1;
        let size = if rot_only { 0x10 } else { 0x20 };
        let mut keys = Vec::with_capacity(nk);
        for k in 0..nk {
            let q = o + 0x10 + k * size;
            let f3 = |at: usize| Ok::<_, String>([f32_at(d, at)?, f32_at(d, at + 4)?, f32_at(d, at + 8)?]);
            let (pos, rot) = if rot_only { (None, f3(q + 4)?) } else { (Some(f3(q + 8)?), f3(q + 0x14)?) };
            keys.push(Key { frame: u16_at(d, q)?, flag: i16::from_le_bytes(rd(d, q + 2)?), pos, rot });
        }
        joints.push(Some(Joint { flag, child, next, keys }));
    }
    Ok(Motion { bone_type, stp_table, flags, range, joints })
}

/// A `MotionPack` record: `u32 count`, then `count × {motion id, offset of the GIM, 0}`.
pub fn pack_items(pk: &[u8]) -> Result<Vec<(u32, usize)>, String> {
    let n = u32_at(pk, 0)? as usize;
    (0..n).map(|i| Ok((u32_at(pk, 4 + 12 * i)?, u32_at(pk, 8 + 12 * i)? as usize))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_only_joint_has_short_keys() {
        let mut d = vec![0u8; 0x100];
        d[..8].copy_from_slice(MAGIC);
        d[8..10].copy_from_slice(&1u16.to_le_bytes());
        d[10..12].copy_from_slice(&1u16.to_le_bytes());
        d[12..16].copy_from_slice(&0x20u32.to_le_bytes());
        d[0x10] = 0;
        d[0x20..0x22].copy_from_slice(&2u16.to_le_bytes());
        d[0x24..0x28].copy_from_slice(&9u32.to_le_bytes());
        d[0x30..0x34].copy_from_slice(&0x40u32.to_le_bytes());
        d[0x38..0x3C].copy_from_slice(&0u32.to_le_bytes());
        d[0x40..0x44].copy_from_slice(&1i32.to_le_bytes());
        d[0x46..0x48].copy_from_slice(&2u16.to_le_bytes());
        d[0x50..0x52].copy_from_slice(&0u16.to_le_bytes());
        d[0x54..0x58].copy_from_slice(&1.5f32.to_le_bytes());
        d[0x60..0x62].copy_from_slice(&3u16.to_le_bytes());
        d[0x64..0x68].copy_from_slice(&2.5f32.to_le_bytes());
        let m = parse(&d).unwrap();
        assert_eq!((m.range, m.joints.len()), (9, 2));
        assert!(m.joints[1].is_none());
        let j = m.joints[0].as_ref().unwrap();
        assert_eq!(j.keys.len(), 2);
        assert_eq!((j.keys[0].pos, j.keys[0].rot[0], j.keys[1].frame, j.keys[1].rot[0]), (None, 1.5, 3, 2.5));
    }
}
