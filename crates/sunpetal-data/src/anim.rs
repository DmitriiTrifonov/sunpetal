//! The `ANIM` section (У2.3): PS2 motions as raw keys — per joint the keys of frame, position and rotation as
//! the game stores them (`ps2::gim`) — plus the table that says which motion a scene's `MotionID` is. The player
//! (`ps2::pose`) samples them; nothing here interpolates.
//!
//! ```text
//! u32 version (1), u32 motions, u32 ids
//! motions × { [u8;5] key (first 10 hex digits of the SHA-1 of the motion's bytes), u16 bone_type, u16 flags, u32 range,
//!             u16 n_std, u8 stp_table[n_std], u16 joints,
//!             joints × { u8 present, if present: i32 flag, u8 child, u8 next, u32 keys,
//!                        keys × { u16 frame, i16 flag, f32 pos[3] unless flag == 1, f32 rot[3] } } }
//! ids × { u8 scene name length, scene name, u32 motion id, [u8;5] key }
//! ```
use crate::ps2::gim::{Joint, Key, Motion};

pub const VERSION: u32 = 1;
pub type MotionKey = [u8; 5];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Anim {
    pub motions: Vec<(MotionKey, Motion)>,
    /// `(scene, MotionID in the scene's pack, motion)`.
    pub ids: Vec<(String, u32, MotionKey)>,
}

/// The key from the 10 hex digits the reference tables use (`models/…`, `motions.tsv`).
pub fn key_from_hex(h: &str) -> Option<MotionKey> {
    if h.len() != 10 {
        return None;
    }
    let mut k = [0u8; 5];
    for (i, b) in k.iter_mut().enumerate() {
        *b = u8::from_str_radix(h.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(k)
}

impl Anim {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::new();
        for n in [VERSION, self.motions.len() as u32, self.ids.len() as u32] {
            o.extend_from_slice(&n.to_le_bytes());
        }
        for (key, m) in &self.motions {
            o.extend_from_slice(key);
            o.extend_from_slice(&m.bone_type.to_le_bytes());
            o.extend_from_slice(&m.flags.to_le_bytes());
            o.extend_from_slice(&m.range.to_le_bytes());
            o.extend_from_slice(&(m.stp_table.len() as u16).to_le_bytes());
            o.extend_from_slice(&m.stp_table);
            o.extend_from_slice(&(m.joints.len() as u16).to_le_bytes());
            for j in &m.joints {
                let Some(j) = j else {
                    o.push(0);
                    continue;
                };
                o.push(1);
                o.extend_from_slice(&j.flag.to_le_bytes());
                o.extend_from_slice(&[j.child, j.next]);
                o.extend_from_slice(&(j.keys.len() as u32).to_le_bytes());
                for k in &j.keys {
                    o.extend_from_slice(&k.frame.to_le_bytes());
                    o.extend_from_slice(&k.flag.to_le_bytes());
                    for v in k.pos.iter().flatten().chain(k.rot.iter()) {
                        o.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
        }
        for (scene, id, key) in &self.ids {
            o.push(scene.len() as u8);
            o.extend_from_slice(scene.as_bytes());
            o.extend_from_slice(&id.to_le_bytes());
            o.extend_from_slice(key);
        }
        o
    }

    pub fn decode(d: &[u8]) -> Result<Anim, String> {
        let mut r = Reader { d, at: 0 };
        if r.u32()? != VERSION {
            return Err("ANIM: unknown version".into());
        }
        let (nm, ni) = (r.u32()? as usize, r.u32()? as usize);
        // every motion takes at least 15 bytes and every id 10: a count the bytes cannot hold is a damaged file
        if nm > d.len() / 15 || ni > d.len() / 10 {
            return Err("ANIM: counts exceed the data".into());
        }
        let mut motions = Vec::with_capacity(nm);
        for _ in 0..nm {
            let key: MotionKey = r.take(5)?.try_into().unwrap();
            let (bone_type, flags, range) = (r.u16()?, r.u16()?, r.u32()?);
            let n_std = r.u16()? as usize;
            let stp_table = r.take(n_std)?.to_vec();
            let nj = r.u16()? as usize;
            let mut joints = Vec::with_capacity(nj);
            for _ in 0..nj {
                if r.u8()? == 0 {
                    joints.push(None);
                    continue;
                }
                let flag = r.u32()? as i32;
                let (child, next) = (r.u8()?, r.u8()?);
                let nk = r.u32()? as usize;
                let size = if flag == 1 { 16 } else { 28 };
                if nk > (d.len() - r.at) / size {
                    return Err("ANIM: keys exceed the data".into());
                }
                let mut keys = Vec::with_capacity(nk);
                for _ in 0..nk {
                    let (frame, kf) = (r.u16()?, r.u16()? as i16);
                    let f3 = |r: &mut Reader| -> Result<[f32; 3], String> { Ok([r.f32()?, r.f32()?, r.f32()?]) };
                    let pos = if flag == 1 { None } else { Some(f3(&mut r)?) };
                    let rot = f3(&mut r)?;
                    if pos.iter().flatten().chain(rot.iter()).any(|v| !v.is_finite()) {
                        return Err("ANIM: a key is not a number".into());
                    }
                    keys.push(Key { frame, flag: kf, pos, rot });
                }
                joints.push(Some(Joint { flag, child, next, keys }));
            }
            motions.push((key, Motion { bone_type, stp_table, flags, range, joints }));
        }
        let mut ids = Vec::with_capacity(ni);
        for _ in 0..ni {
            let n = r.u8()? as usize;
            let scene = String::from_utf8(r.take(n)?.to_vec()).map_err(|_| "ANIM: a scene name is not text")?;
            let id = r.u32()?;
            let key: MotionKey = r.take(5)?.try_into().unwrap();
            if !motions.iter().any(|m| m.0 == key) {
                return Err("ANIM: an id names a motion that is not there".into());
            }
            ids.push((scene, id, key));
        }
        if r.at != d.len() {
            return Err("ANIM: bytes left over".into());
        }
        Ok(Anim { motions, ids })
    }

    pub fn motion(&self, scene: &str, id: u32) -> Option<&Motion> {
        let key = self.ids.iter().find(|i| i.0 == scene && i.1 == id)?.2;
        self.motions.iter().find(|m| m.0 == key).map(|m| &m.1)
    }
}

struct Reader<'a> {
    d: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self.d.get(self.at..self.at + n).ok_or("ANIM: cut short")?;
        self.at += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_bits(self.u32()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Anim {
        let key = key_from_hex("0123456789").unwrap();
        let m = Motion {
            bone_type: 1,
            stp_table: vec![0, 1, 0xFF],
            flags: 0,
            range: 9,
            joints: vec![
                Some(Joint { flag: 0, child: 1, next: 255, keys: vec![Key { frame: 0, flag: 0, pos: Some([1.0, 2.0, 3.0]), rot: [0.1, 0.2, 0.3] }] }),
                None,
                Some(Joint { flag: 1, child: 255, next: 255, keys: vec![Key { frame: 4, flag: -1, pos: None, rot: [0.5, 0.0, -0.5] }] }),
            ],
        };
        Anim { motions: vec![(key, m)], ids: vec![("R00".into(), 366, key)] }
    }

    #[test]
    fn a_table_of_motions_round_trips() {
        let a = sample();
        assert_eq!(Anim::decode(&a.encode()).unwrap(), a);
        assert_eq!(a.motion("R00", 366).unwrap().range, 9);
        assert!(a.motion("R00", 1).is_none());
    }

    #[test]
    fn damaged_bytes_are_refused() {
        let b = sample().encode();
        assert!(Anim::decode(&b[..b.len() - 1]).is_err());
        let mut more = b.clone();
        more.push(0);
        assert!(Anim::decode(&more).is_err());
        let mut v = b.clone();
        v[0] = 9;
        assert!(Anim::decode(&v).is_err());
        let mut huge = b.clone();
        huge[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Anim::decode(&huge).is_err());
        // an id that names no motion
        let mut a = sample();
        a.ids[0].2 = [9; 5];
        assert!(Anim::decode(&a.encode()).is_err());
        // a key that is not a number
        let mut nan = sample();
        nan.motions[0].1.joints[0].as_mut().unwrap().keys[0].rot[0] = f32::NAN;
        assert!(Anim::decode(&nan.encode()).is_err());
    }
}
