//! The native choice of the next script command (K-192, step E2.1, `docs/NATIVE-ENGINE-PLAN.md`): the game's
//! `0x020475E0` as host code, behind `SUNPETAL_NATIVE_STEP=1`. The cursor `[0x0207596C+0x64]` is the position key
//! of the next command; the function takes the first command of the table `0x02074BEC` (count `0x020740A0`) whose key is
//! not below the cursor, moves the cursor to the key of the one after it (or -1) and answers the command's address.
use crate::abi::Cpu;
use crate::services::{mem_read32, mem_write32};

const CURSOR: u32 = 0x0207_596C + 0x64;
const COUNT: u32 = 0x0207_40A0;
const TABLE: u32 = 0x0207_4BEC;

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

pub fn wanted() -> bool {
    std::env::var("SUNPETAL_NATIVE_STEP").is_ok_and(|v| v == "1")
}

pub fn install() {
    crate::machine(|m| m.override_fn(9, 0x0204_75E1, select));
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
    fn nothing_past_the_end_and_nothing_at_minus_one() {
        assert_eq!(pick(10, &[3, 5, 9]), (None, 2));
        assert_eq!(pick(-1, &[3, 5, 9]), (None, 0));
        assert_eq!(pick(1, &[]), (None, 0));
    }
}
