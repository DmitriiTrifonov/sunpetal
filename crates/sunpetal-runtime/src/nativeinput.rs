//! The native key sampler (K-189, step 189.5, `docs/NATIVE-DIALOG-DRAW.md`): the first half of the game's per-frame
//! input routine `0x0202C374` (Thumb) as host code. It takes the keys (own `REG_KEYINPUT` and the ARM7's copy of the
//! extra keys), keeps the previous word, and writes the pressed-now word the script step reads (`0x0206CA72`, A is bit 0)
//! and the edge word; the key-repeat helper `0x0202C3E4` stays the game's. Behind `SUNPETAL_NATIVE_INPUT=1`.
use crate::abi::Cpu;
use crate::services::{mem_read16, mem_write16};

const STATE: u32 = 0x0206_CA6C; // [+0] scratch, [+2] previous, [+6] edge, [+8] held
const KEYINPUT: u32 = 0x0400_0130;
const EXTKEYS_COPY: u32 = 0x027F_FFA8;
const MASK: u32 = 0x2FFF;
const REPEAT_STATE: u32 = 0x0206_CA98;
const REPEAT_OUT: u32 = 0x0206_CA70;
const REPEAT_KEY: u32 = 0x0202_C3E5;

/// The words one sample leaves: held keys (pressed = 1), and what changed to pressed since the previous sample.
pub fn sample(keyinput: u32, ext: u32, prev_held: u32, scratch: u32) -> (u32, u32, u32) {
    let pressed = (MASK & !(keyinput | ext)) & 0xFFFF;
    let held = pressed | (scratch & 0xFFFF);
    let edge = !prev_held & held & 0xFFFF;
    (held, edge, pressed)
}

unsafe extern "C" fn hook(c: *mut Cpu) {
    let lr = (*c).r[14];
    let (key, ext) = (mem_read16(c, KEYINPUT), mem_read16(c, EXTKEYS_COPY));
    let (prev, scratch) = (mem_read16(c, STATE + 8), mem_read16(c, STATE));
    mem_write16(c, STATE + 2, prev);
    let (held, edge, _) = sample(key, ext, prev, scratch);
    mem_write16(c, STATE + 8, held);
    mem_write16(c, STATE, 0);
    mem_write16(c, STATE + 6, edge);
    // the key-repeat helper, as the game calls it
    (*c).r[0] = REPEAT_STATE;
    (*c).r[1] = held;
    (*c).r[2] = REPEAT_OUT;
    (*c).r[14] = 0x0202_C3AD;
    crate::call(&mut *c, REPEAT_KEY);
    (*c).r[0] = 1;
    (*c).pc = if lr & 1 != 0 { lr } else { lr & !3 };
}

pub fn wanted() -> bool {
    std::env::var("SUNPETAL_NATIVE_INPUT").is_ok_and(|v| v == "1")
}

pub fn install() {
    crate::machine(|m| m.override_fn(9, 0x0202_C375, hook));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_is_a_set_bit_once() {
        // KEYINPUT is active low: bit 0 clear = A down
        let (held, edge, pressed) = sample(0x3FFE, 0, 0, 0);
        assert_eq!((held, edge, pressed), (1, 1, 1));
        let (held2, edge2, _) = sample(0x3FFE, 0, held, 0);
        assert_eq!((held2, edge2), (1, 0), "held, no new edge");
    }

    #[test]
    fn the_mask_keeps_to_the_known_keys_and_the_two_words_are_or_ed() {
        // the sampler ORs the two words as the game does, then flips and masks them
        assert_eq!(sample(0x3BFF, 0, 0, 0).0, 1 << 10);
        assert_eq!(sample(0x3FFF, 0, 0, 0).0, 0);
        assert_eq!(sample(0xFFFF, 0, 0, 0).0, 0, "bits outside the mask are ignored");
        assert_eq!(sample(0x3FFF, 0, 0, 0x0003).0, 3, "the scratch word joins the held keys");
    }
}
