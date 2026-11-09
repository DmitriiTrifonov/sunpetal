//! The native choice of the next script command (K-192, step E2.1, `docs/NATIVE-ENGINE-PLAN.md`): the game's
//! `0x020475E0` as host code, behind `SUNPETAL_NATIVE_STEP=1`. The cursor `[0x0207596C+0x64]` is the position key
//! of the next command; the function takes the first command of the table `0x02074BEC` (count `0x020740A0`) whose key is
//! not below the cursor, moves the cursor to the key of the one after it (or -1) and answers the command's address.
//! The flow commands `JUMP`, `SETVALUE`, `ADDVALUE`, `CHANGEBYVALUE` (E2.2) are here too: they move the cursor by the
//! label table `0x020747EC` (8 bytes a label) and keep the script variables `0x020742EC`.
use crate::abi::Cpu;
use crate::services::{mem_read32, mem_write32};

const CURSOR: u32 = 0x0207_596C + 0x64;
const COUNT: u32 = 0x0207_40A0;
const TABLE: u32 = 0x0207_4BEC;
const CMD_PTR: u32 = 0x0207_596C + 0x1C; // [..] = the running command: [+8] first parameter, [+0xC] second...
const LABELS: u32 = 0x0207_47EC;
const VARS: u32 = 0x0207_42EC;

/// The pick over `keys` (position keys of the table, in order): `(index, new cursor)` and the safe points the game's
/// loop passes (one per step back), none if the cursor is -1 or the table is empty.
pub fn pick(cursor: i32, keys: &[i32]) -> (Option<(usize, i32)>, u32) {
    if cursor == -1 {
        return (None, 0);
    }
    match keys.iter().position(|&k| cursor <= k) {
        Some(i) => (Some((i, keys.get(i + 1).copied().unwrap_or(-1))), i as u32),
        None => (None, keys.len().saturating_sub(1) as u32),
    }
}

unsafe extern "C" fn select(c: *mut Cpu) {
    let cursor = mem_read32(c, CURSOR) as i32;
    let n = mem_read32(c, COUNT) as i32;
    let cmd = |i: usize| mem_read32(c, TABLE + 4 * i as u32);
    let keys: Vec<i32> = if cursor == -1 || n <= 0 { Vec::new() } else { (0..n as usize).map(|i| mem_read32(c, cmd(i) + 4) as i32).collect() };
    let (hit, polls) = pick(cursor, &keys);
    (*c).r[0] = match hit {
        Some((i, next)) => {
            mem_write32(c, CURSOR, next as u32);
            cmd(i)
        }
        None => 0,
    };
    for _ in 0..polls {
        (*c).budget -= 1;
        if (*c).budget < 0 {
            crate::services::arm_poll(c);
        }
    }
    let lr = (*c).r[14];
    (*c).pc = if lr & 1 != 0 { lr } else { lr & !3 };
}

unsafe fn param(c: *mut Cpu, off: u32) -> u32 {
    mem_read32(c, mem_read32(c, CMD_PTR).wrapping_add(off))
}

unsafe fn label_key(c: *mut Cpu, label: u32) -> u32 {
    mem_read32(c, LABELS.wrapping_add(label << 3))
}

unsafe fn ret(c: *mut Cpu, polls: u32) {
    for _ in 0..polls {
        (*c).budget -= 1;
        if (*c).budget < 0 {
            crate::services::arm_poll(c);
        }
    }
    let lr = (*c).r[14];
    (*c).pc = if lr & 1 != 0 { lr } else { lr & !3 };
}

/// `CHANGEBYVALUE`: the first branch whose bound is not below the variable; the label of it. And the loop's polls.
pub fn branch(value: i32, bounds: &[i32]) -> (Option<usize>, u32) {
    match bounds.iter().position(|&b| value <= b) {
        Some(i) => (Some(i), i as u32),
        None => (None, bounds.len().saturating_sub(1) as u32),
    }
}

unsafe extern "C" fn jump(c: *mut Cpu) {
    let k = label_key(c, param(c, 8));
    mem_write32(c, CURSOR, k);
    ret(c, 0);
}

unsafe extern "C" fn set_value(c: *mut Cpu) {
    mem_write32(c, VARS + 4 * param(c, 8), param(c, 0xC));
    ret(c, 0);
}

unsafe extern "C" fn add_value(c: *mut Cpu) {
    let a = VARS + 4 * param(c, 8);
    let v = mem_read32(c, a).wrapping_add(param(c, 0xC));
    mem_write32(c, a, v);
    ret(c, 0);
}

unsafe extern "C" fn change_by_value(c: *mut Cpu) {
    let value = mem_read32(c, VARS + 4 * param(c, 8)) as i32;
    let n = param(c, 0xC) as i32;
    let bounds: Vec<i32> = (0..n.max(0) as u32).map(|i| param(c, 0x10 + 4 * i) as i32).collect();
    let (hit, polls) = branch(value, &bounds);
    if let Some(i) = hit {
        let k = label_key(c, param(c, 0x20 + 4 * i as u32));
        mem_write32(c, CURSOR, k);
    }
    ret(c, polls);
}

pub fn wanted() -> bool {
    std::env::var("SUNPETAL_NATIVE_STEP").is_ok_and(|v| v == "1")
}

pub fn install() {
    crate::machine(|m| {
        m.override_fn(9, 0x0204_75E1, select);
        m.override_fn(9, 0x0204_80B1, jump);
        m.override_fn(9, 0x0204_859D, set_value);
        m.override_fn(9, 0x0204_85B5, add_value);
        m.override_fn(9, 0x0204_85D5, change_by_value);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_key_not_below_the_cursor_wins_and_the_cursor_moves_on() {
        assert_eq!(pick(5, &[3, 5, 9]), (Some((1, 9)), 1));
        assert_eq!(pick(2, &[3, 5, 9]), (Some((0, 5)), 0));
        assert_eq!(pick(9, &[3, 5, 9]), (Some((2, -1)), 2), "the last one leaves the cursor at -1");
    }

    #[test]
    fn a_branch_takes_the_first_bound_not_below_the_value() {
        assert_eq!(branch(3, &[2, 5, 9]), (Some(1), 1));
        assert_eq!(branch(2, &[2, 5, 9]), (Some(0), 0));
        assert_eq!(branch(10, &[2, 5, 9]), (None, 2), "above every bound: no branch");
        assert_eq!(branch(1, &[]), (None, 0));
    }

    #[test]
    fn nothing_past_the_end_and_nothing_at_minus_one() {
        assert_eq!(pick(10, &[3, 5, 9]), (None, 2));
        assert_eq!(pick(-1, &[3, 5, 9]), (None, 0));
        assert_eq!(pick(1, &[]), (None, 0));
    }
}
