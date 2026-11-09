//! The native `END` of the script (K-197, step E2b, `docs/NATIVE-ENGINE-PLAN.md`): the game's `0x02048250` as host
//! code, behind `SUNPETAL_NATIVE_END=1`. `END` ends a scene: it lets go of what the scene held — twenty object slots
//! (a destroyer and the allocator's free for each, then the slot is zeroed), the command table, the label table, two
//! buffers — and resets a few words. The work of every piece stays the game's own function; the order, the tests of the
//! slots and the safe points (one before each call, one for each step back of a loop) are the same as the game's.
use crate::abi::Cpu;
use crate::nativechar::spend;
use crate::services::{mem_read32, mem_write32};

const FREE: u32 = 0x0202_BFB5; // the allocator's free (Thumb)
const FREE2: u32 = 0x0202_BFC1; // the free of a plain block
const S596C: u32 = 0x0207_596C;
const S58EC: u32 = 0x0207_58EC;
const S59EC: u32 = 0x0207_59EC;

/// `(base, offset, destroyer, return address of its call, return address of the free call)`, in the game's order.
const SLOTS: [(u32, u32, u32, u32, u32); 20] = [
    (S596C, 0x5C, 0x0205_0A28, 0x0204_828D, 0x0204_8293),
    (S58EC, 0x6C, 0x0205_0A28, 0x0204_82A9, 0x0204_82AF),
    (S58EC, 0x7C, 0x0205_8FB8, 0x0204_82C5, 0x0204_82CB),
    (S58EC, 0x10, 0x0205_0A28, 0x0204_82E1, 0x0204_82E7),
    (S58EC, 0x38, 0x0205_0A28, 0x0204_82FD, 0x0204_8303),
    (S596C, 0x2C, 0x0205_2AA4, 0x0204_8319, 0x0204_831F),
    (S58EC, 0x40, 0x0205_3AF8, 0x0204_8335, 0x0204_833B),
    (S596C, 0x24, 0x0205_0A28, 0x0204_8351, 0x0204_8357),
    (S596C, 0x20, 0x0205_4874, 0x0204_836D, 0x0204_8373),
    (S596C, 0x44, 0x0205_7614, 0x0204_8389, 0x0204_838F),
    (S596C, 0x30, 0x0205_8070, 0x0204_83A5, 0x0204_83AB),
    (S58EC, 0x14, 0x0205_4D30, 0x0204_83C1, 0x0204_83C7),
    (S596C, 0x14, 0x0205_4FD0, 0x0204_83DD, 0x0204_83E3),
    (S596C, 0x10, 0x0205_5300, 0x0204_83F9, 0x0204_83FF),
    (S59EC, 0x10, 0x0205_0A28, 0x0204_8415, 0x0204_841B),
    (S596C, 0x0C, 0x0205_58D0, 0x0204_8431, 0x0204_8437),
    (S59EC, 0x38, 0x0205_0A28, 0x0204_844D, 0x0204_8453),
    (S596C, 0x04, 0x0205_5FEC, 0x0204_8469, 0x0204_846F),
    (S596C, 0x38, 0x0205_7EF8, 0x0204_8485, 0x0204_848B),
    (S58EC, 0x3C, 0x0205_8378, 0x0204_84A1, 0x0204_84A7),
];

const COUNT: u32 = 0x0207_40A0;
const TABLE: u32 = 0x0207_4BEC;
const LABELS: u32 = 0x0207_47EC;

/// A call into the game with `r0`, `r1`, `r2`: one safe point first, as the recompiled `bl`.
unsafe fn call(c: *mut Cpu, key: u32, ret: u32, r0: u32, r1: u32, r2: u32) {
    (*c).r[0] = r0;
    (*c).r[1] = r1;
    (*c).r[2] = r2;
    (*c).r[14] = ret;
    spend(c, 1);
    crate::call(&mut *c, key);
}

/// The slot `[addr]`, if it holds something: its destroyer, the free, then zero.
unsafe fn let_go(c: *mut Cpu, base: u32, off: u32, f: u32, ret: u32, ret_free: u32) {
    let v = mem_read32(c, base + off);
    if v != 0 {
        call(c, f | 1, ret, v, 0, 0);
        call(c, FREE, ret_free, v, 0, 0);
        mem_write32(c, base + off, 0);
    }
}

/// A plain block at the word `addr`: freed and zeroed.
unsafe fn free_word(c: *mut Cpu, addr: u32, ret: u32) {
    let v = mem_read32(c, addr);
    if v != 0 {
        call(c, FREE2, ret, v, 0, 0);
        mem_write32(c, addr, 0);
    }
}

unsafe extern "C" fn end(c: *mut Cpu) {
    if std::env::var_os("SUNPETAL_NATIVE_END_LOG").is_some() {
        eprintln!("native END");
    }
    let (lr, saved) = ((*c).r[14], [(*c).r[3], (*c).r[4], (*c).r[5], (*c).r[6], (*c).r[7]]);
    // the push {r3-r7, lr}: the callees see the stack as the game's code leaves it
    let sp = (*c).r[13];
    let a = sp.wrapping_sub(24) & !3;
    for (i, v) in saved.iter().enumerate() {
        mem_write32(c, a + 4 * i as u32, *v);
    }
    mem_write32(c, a + 20, lr);
    (*c).r[13] = sp.wrapping_sub(24);

    let cur = S596C + 0x3C;
    call(c, 0x0202_CA51, 0x0204_825B, mem_read32(c, cur), 0, 0);
    let mut release = (*c).r[0] == 3;
    if !release {
        call(c, 0x0202_CA51, 0x0204_8267, mem_read32(c, cur), 0, 0);
        release = (*c).r[0] == 4;
    }
    if release {
        call(c, 0x0202_13F9, 0x0204_8273, mem_read32(c, cur), 0, 0);
    }
    call(c, 0x0205_30F5, 0x0204_827D, mem_read32(c, S596C + 0x68), 0, 0);

    for (base, off, f, ret, ret_free) in SLOTS {
        let_go(c, base, off, f, ret, ret_free);
    }

    // the command table: while the count (read again at each step) is above the index
    if mem_read32(c, COUNT) as i32 > 0 {
        let mut i = 0u32;
        loop {
            let v = mem_read32(c, TABLE + 4 * i);
            if v != 0 {
                call(c, 0x0203_EF75, 0x0204_84C9, v, 0, 0);
                call(c, FREE, 0x0204_84CF, v, 0, 0);
                mem_write32(c, TABLE + 4 * i, 0);
            }
            i += 1;
            if (i as i32) < mem_read32(c, COUNT) as i32 {
                spend(c, 1);
            } else {
                break;
            }
        }
    }
    // the label table: 128 entries of 8 bytes, a block at +4
    for i in 0..0x80u32 {
        let a = LABELS + 8 * i + 4;
        let v = mem_read32(c, a);
        if v != 0 {
            call(c, FREE2, 0x0204_84ED, v, 0, 0);
            mem_write32(c, a, 0);
        }
        if i + 1 < 0x80 {
            spend(c, 1);
        }
    }
    free_word(c, 0x0207_4098, 0x0204_8503);
    free_word(c, 0x0207_4094, 0x0204_8515);

    mem_write32(c, 0x0206_577C, (-2i32) as u32);
    mem_write32(c, S59EC + 0x30, 0);
    mem_write32(c, S59EC + 0x48, 0);
    let p = mem_read32(c, S58EC + 0x4C);
    if p != 0 {
        mem_write32(c, p, 2 << 10);
        mem_write32(c, p + 4, 0xDDB);
    }
    call(c, 0x0202_49B9, 0x0204_856D, 0, 0, 0);
    let r = (*c).r[0];
    if r != 0 {
        call(c, 0x0202_84ED, 0x0204_8577, r, 0, 0);
    }
    if mem_read32(c, S58EC + 0x5C) != 0 {
        call(c, 0x0200_51CC, 0x0204_858B, 0x0206_57B4, 0x1A << 4, 0x20);
        mem_write32(c, S58EC + 0x5C, 0);
    }

    // the pop {r3-r7, pc}
    (*c).r[13] = sp;
    [(*c).r[3], (*c).r[4], (*c).r[5], (*c).r[6], (*c).r[7]] = saved;
    (*c).pc = if lr & 1 != 0 { lr } else { lr & !3 };
}

pub fn wanted() -> bool {
    std::env::var("SUNPETAL_NATIVE_END").is_ok_and(|v| v == "1")
}

pub fn install() {
    crate::machine(|m| m.override_fn(9, 0x0204_8251, end));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slot_table_matches_the_games_code() {
        assert_eq!(SLOTS.len(), 20);
        // every return address is a Thumb one inside the function, in the game's order
        let mut last = 0;
        for (_, _, f, r1, r2) in SLOTS {
            assert!(f & 1 == 0 && r1 & 1 == 1 && r2 == r1 + 6 && r1 > last);
            assert!((0x0204_8250..0x0204_8590).contains(&r1));
            last = r2;
        }
    }
}
