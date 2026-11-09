//! The native character commands (K-190, step E1.3, `docs/NATIVE-ENGINE-PLAN.md`): the character finder
//! `0x020245B0` and the script handlers `SETCHARPOS`, `SETDIRECTION`, `SETMOTION` as host code, behind
//! `SUNPETAL_NATIVE_CHARS=1`. The characters are 16 records of `0x1E80` bytes at `[0x0206B460] + 4`, the id at `+0x5C`;
//! the finder also marks the id as met in the saved flag words `0x0209C9CC…` (`0x02024B6C`), that is part of its job.
//! `SETMOTION` hands the motion itself to the game's functions (`0x02028BC0`, `0x02028BF8`): they are the next step.
use crate::abi::Cpu;
use crate::services::{mem_read32, mem_write32};

const BASE_PTR: u32 = 0x0206_B460; // [..] = base of the table: [base] is the first record's pointer, records at base + 4
const STRIDE: u32 = 0x1E80;
const COUNT: u32 = 16;
const ID_OFF: u32 = 0x5C;
const SEEN: u32 = 0x0209_C9CC; // 64-bit masks for ids 1..=64 and 65..=128
const CMD_PTR: u32 = 0x0207_596C + 0x1C; // [..] = the running command: [+8] id, [+0xC..] parameters
const ALL_CHARS: u32 = 0x0207_6308; // the finder's answer for id -1

/// The 16-bit angle of a degree value the script gives (`SETDIRECTION`): `deg · 0x3243 / 180`, scaled by the game's
/// constant `0x28BE60DB9391` and rounded; the carry of the arithmetic shift of the quotient joins the rounding.
pub fn direction(deg: i32) -> u32 {
    let q = ((deg as u32).wrapping_mul(0x3243)) as i32 / 180;
    let hi = ((q as i64 as u64).wrapping_mul(0x28BE_60DB_9391) >> 32) as u32;
    let r = hi.wrapping_add(0x800).wrapping_add(((q as u32) >> 30) & 1);
    (r << 4) >> 16
}

/// The bit of an id in the two `met` masks, `(word index, bit)`, none outside 1..=128.
pub fn seen_bit(id: i32) -> Option<(u32, u32)> {
    match id {
        1..=64 => Some((0, id as u32 - 1)),
        65..=128 => Some((2, id as u32 - 65)),
        _ => None,
    }
}

unsafe fn mark_seen(c: *mut Cpu, id: i32) {
    if let Some((w, bit)) = seen_bit(id) {
        let a = SEEN + w * 4;
        let v = (mem_read32(c, a) as u64) | (mem_read32(c, a + 4) as u64) << 32 | 1u64 << bit;
        mem_write32(c, a, v as u32);
        mem_write32(c, a + 4, (v >> 32) as u32);
    }
}

/// `0x020245B0`: the record of the character with this id (0 if none) and the safe points the game's function
/// would have passed on the way (its call of `0x02024B6C`, the one inside it for a met-mask id, each step back of the
/// search). The game counts time by them, so the host spends the same.
unsafe fn find(c: *mut Cpu, id: u32) -> (u32, u32) {
    mark_seen(c, id as i32);
    let polls = 1 + seen_bit(id as i32).is_some() as u32;
    let base = mem_read32(c, BASE_PTR);
    if id == 0 {
        return (mem_read32(c, base), polls);
    }
    if id == u32::MAX {
        return (ALL_CHARS, polls);
    }
    // as the game: the first record's id is read through its pointer once, the others by their place
    if mem_read32(c, mem_read32(c, base).wrapping_add(ID_OFF)) == id {
        return (mem_read32(c, base), polls);
    }
    for i in 0..COUNT {
        if mem_read32(c, base.wrapping_add(0x60 + i * STRIDE)) == id {
            return (base.wrapping_add(4 + i * STRIDE), polls + i);
        }
    }
    (0, polls + COUNT - 1)
}

/// Spends `n` safe points as the recompiled code does: the budget runs down and `arm_poll` takes the time.
unsafe fn spend(c: *mut Cpu, n: u32) {
    for _ in 0..n {
        (*c).budget -= 1;
        if (*c).budget < 0 {
            crate::services::arm_poll(c);
        }
    }
}

unsafe fn ret(c: *mut Cpu) {
    let lr = (*c).r[14];
    (*c).pc = if lr & 1 != 0 { lr } else { lr & !3 };
}

unsafe fn param(c: *mut Cpu, off: u32) -> u32 {
    mem_read32(c, mem_read32(c, CMD_PTR).wrapping_add(off))
}

unsafe extern "C" fn finder(c: *mut Cpu) {
    let id = (*c).r[0];
    let (rec, polls) = find(c, id);
    (*c).r[0] = rec;
    spend(c, polls);
    ret(c);
}

unsafe extern "C" fn set_char_pos(c: *mut Cpu) {
    let (ch, polls) = find(c, param(c, 8));
    if ch != 0 {
        for (i, off) in [0xC, 0x10, 0x14].into_iter().enumerate() {
            mem_write32(c, ch + 0x18 + 4 * i as u32, param(c, off));
        }
    }
    spend(c, 1 + polls); // the call of the finder
    ret(c);
}

unsafe extern "C" fn set_direction(c: *mut Cpu) {
    let angle = direction(param(c, 0xC) as i32);
    let (ch, polls) = find(c, param(c, 8));
    if ch != 0 {
        mem_write32(c, ch + 0x4C, angle);
    }
    spend(c, 3 + polls); // the division, the 64-bit product, the finder
    ret(c);
}

unsafe extern "C" fn set_motion(c: *mut Cpu) {
    let lr = (*c).r[14];
    let (id, motion, flag) = (param(c, 8), param(c, 0xC), (param(c, 0x10) == 1) as u32);
    let (ch, polls) = find(c, id);
    spend(c, 1 + polls);
    if ch != 0 {
        // the game's motion functions: `motion >= 100` is the other family
        let target = if motion as i32 >= 100 { 0x0202_8BF9 } else { 0x0202_8BC1 };
        (*c).r[0] = ch;
        (*c).r[1] = 0;
        (*c).r[2] = motion;
        (*c).r[3] = flag;
        (*c).r[14] = 0x0204_8795; // where the handler's own call returns
        spend(c, 1);
        crate::call(&mut *c, target);
    }
    (*c).r[14] = lr;
    ret(c);
}

/// `SUNPETAL_NATIVE_CHARS=1` is all of them; a comma list (`finder,pos,dir,motion`) takes some, to bisect a mismatch.
fn chosen() -> Vec<String> {
    match std::env::var("SUNPETAL_NATIVE_CHARS") {
        Ok(v) if v == "1" => ["finder", "pos", "dir", "motion"].map(String::from).to_vec(),
        Ok(v) if !v.is_empty() && v != "0" => v.split(',').map(|s| s.trim().to_string()).collect(),
        _ => Vec::new(),
    }
}

pub fn wanted() -> bool {
    !chosen().is_empty()
}

pub fn install() {
    let table: [(&str, u32, unsafe extern "C" fn(*mut Cpu)); 4] =
        [("finder", 0x0202_45B1, finder), ("pos", 0x0204_863D, set_char_pos), ("dir", 0x0204_975D, set_direction), ("motion", 0x0204_875D, set_motion)];
    let on = chosen();
    crate::machine(|m| {
        for (name, key, f) in table {
            if on.iter().any(|o| o == name) {
                m.override_fn(9, key, f);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degrees_become_sixteen_bit_angles() {
        // the game's own rounding is a little short of 65536/360 per degree (90 -> 16381): near, not equal
        let near = |a: u32, b: u32| a.abs_diff(b) <= 8;
        assert_eq!(direction(0), 0);
        assert!(near(direction(90), 0x4000), "{}", direction(90));
        assert!(near(direction(180), 0x8000), "{}", direction(180));
    }

    #[test]
    fn the_met_masks_cover_one_to_128() {
        assert_eq!(seen_bit(0), None);
        assert_eq!(seen_bit(1), Some((0, 0)));
        assert_eq!(seen_bit(64), Some((0, 63)));
        assert_eq!(seen_bit(65), Some((2, 0)));
        assert_eq!(seen_bit(128), Some((2, 63)));
        assert_eq!(seen_bit(129), None);
        assert_eq!(seen_bit(-1), None);
    }
}
