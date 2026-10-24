//! The BIOS: the state a boot leaves, the SWIs, the interrupt entry.
//!
//! The boot is melonDS's direct boot (the oracle's): the programs copied from the cartridge to
//! their load addresses, the header and the firmware's user settings in main RAM's shared area,
//! the chip ID, the CPUs in system mode with the BIOS's stacks. The SWIs are the BIOS's functions
//! done in Rust, as GBATEK describes them. An interrupt is taken as the BIOS takes it: IRQ mode,
//! r0-r3, r12 and the return address pushed on the IRQ stack, the handler the program put at the
//! DTCM's end (ARM9) or 0x0380FFFC (ARM7) called, the frame popped, `subs pc, lr, #4` (a thread
//! switch in the handler changes the frame: `sched::irq_return`).
//!
//! Called from the CPUs' threads with nothing borrowed: they run guest code and may yield.
use crate::abi::{Cpu, BANK_IRQ, BANK_SVC};
use crate::machine::{fatal_msg, machine};
use crate::nds::{nds, Nds};
use crate::sched;
use crate::services::*;

fn ld32(c: &mut Cpu, a: u32) -> u32 {
    unsafe { mem_read32(c, a & !3) }
}
fn ld16(c: &mut Cpu, a: u32) -> u32 {
    unsafe { mem_read16(c, a & !1) }
}
fn ld8(c: &mut Cpu, a: u32) -> u32 {
    unsafe { mem_read8(c, a) }
}
fn st32(c: &mut Cpu, a: u32, v: u32) {
    unsafe { mem_write32(c, a & !3, v) }
}
fn st16(c: &mut Cpu, a: u32, v: u32) {
    unsafe { mem_write16(c, a & !1, v) }
}
fn st8(c: &mut Cpu, a: u32, v: u32) {
    unsafe { mem_write8(c, a, v) }
}

fn rd16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn rd32(b: &[u8], o: usize) -> u32 {
    rd16(b, o) as u32 | (rd16(b, o + 2) as u32) << 16
}

// ---- the boot --------------------------------------------------------------------------------
/// The memory a direct boot leaves: the header and the firmware's settings in main RAM's shared
/// area, the programs at their load addresses. Returns the two entries.
pub(crate) fn direct_boot_memory(n: &mut Nds) -> Result<[u32; 2], String> {
    let rom = n.rom().to_vec();
    let id = n.chip_id();
    let us = n.firmware_user_settings();
    let put = |cpu: i32, a: u32, d: &[u8]| -> Result<(), String> {
        if machine(|m| m.mem.write_bytes(cpu, a, d)) { Ok(()) } else { Err(format!("cannot write {:08X}", a)) }
    };
    let p32 = |a: u32, v: u32| put(9, a, &v.to_le_bytes());
    let p16 = |a: u32, v: u16| put(9, a, &v.to_le_bytes());
    put(9, 0x027F_FE00, &rom[..0x170])?;
    for base in [0x027F_F800u32, 0x027F_FC00] {
        p32(base, id)?;
        p32(base + 4, id)?;
        p16(base + 8, rd16(&rom, 0x15E))?; // the header's CRC
        p16(base + 10, rd16(&rom, 0x6C))?; // the secure area's
    }
    p16(0x027F_F850, 0x5835)?; // the ARM7 BIOS's CRC
    p16(0x027F_FC10, 0x5835)?;
    p16(0x027F_FC30, 0xFFFF)?;
    p16(0x027F_FC40, 0x0001)?; // booted from a cartridge
    p32(0x027F_F864, 0)?;
    p32(0x027F_F868, 0x3FC0 << 3)?; // where the firmware keeps the user settings
    put(9, 0x027F_FC80, &us)?;
    let mut entries = [0; 2];
    for cpu in [9, 7] {
        let h = if cpu == 9 { 0x20 } else { 0x30 };
        let (off, ram, size) = (rd32(&rom, h) as usize, rd32(&rom, h + 8), rd32(&rom, h + 12) as usize);
        if off + size > rom.len() {
            return Err(format!("the ARM{} program lies past the cartridge's end", cpu));
        }
        put(cpu, ram, &rom[off..off + size])?;
        entries[if cpu == 9 { 0 } else { 1 }] = rd32(&rom, h + 4);
    }
    machine(|m| m.overlays_unknown());
    Ok(entries)
}

/// The CPU as a direct boot leaves it: system mode, interrupts masked, the BIOS's stacks.
pub(crate) fn direct_boot_cpu(cpu: i32, entry: u32) {
    let c = unsafe { &mut *sched::cpu_ptr(cpu) };
    c.mode = 0x1F;
    c.i = 1;
    c.f = 1;
    c.r[12] = entry;
    c.r[14] = entry;
    c.r[13] = if cpu == 9 { 0x0300_2F7C } else { 0x0380_FD80 };
    c.bank[BANK_IRQ][0] = if cpu == 9 { 0x0300_3F80 } else { 0x0380_FF80 };
    c.bank[BANK_SVC][0] = if cpu == 9 { 0x0300_3FC0 } else { 0x0380_FFC0 };
}

// ---- interrupts -------------------------------------------------------------------------------
/// Where the BIOS's handler goes on.
const BIOS_RET9: u32 = 0xFFFF_0290;
const BIOS_RET7: u32 = 0x0000_0290;

/// Takes an interrupt (IRQ mode, the user's handler, return).
pub fn irq(c: &mut Cpu) {
    let cpu = c.cpu;
    let cpsr = c.cpsr();
    let ret = sched::irq_marker(cpu) + 4;
    set_mode(c, 0x12);
    c.spsr[BANK_IRQ] = cpsr;
    c.i = 1;
    c.r[14] = ret;
    let mut sp = c.r[13].wrapping_sub(24);
    c.r[13] = sp;
    const SAVED: [usize; 6] = [0, 1, 2, 3, 12, 14];
    for (k, r) in SAVED.iter().enumerate() {
        let v = c.r[*r];
        st32(c, sp + 4 * k as u32, v);
    }
    c.r[0] = if cpu == 9 { machine(|m| m.mem.dtcm_base()) + 0x4000 } else { 0x0400_0000 };
    let h = ld32(c, c.r[0] - 4);
    let back = if cpu == 9 { BIOS_RET9 } else { BIOS_RET7 };
    c.r[14] = back;
    c.pc = 0;
    let key = if cpu == 9 { if h & 1 != 0 { h } else { h & !3 } } else { h & !3 };
    unsafe { arm_call(c, key) };
    if c.pc != back {
        fatal_msg(&format!("ARM{}: the interrupt handler {:08X} returned to {:08X}", cpu, h, c.pc));
    }
    sp = c.r[13];
    for (k, r) in SAVED.iter().enumerate() {
        c.r[*r] = ld32(c, sp + 4 * k as u32);
    }
    c.r[13] = sp + 24;
    let target = c.r[14].wrapping_sub(4);
    restore_cpsr(c, target);
    sched::irq_return(c);
}

// ---- the SWIs ----------------------------------------------------------------------------------
fn intr_wait(c: &mut Cpu, discard: bool, mask: u32) {
    let cpu = c.cpu;
    let flags = if cpu == 9 { machine(|m| m.mem.dtcm_base()) + 0x3FF8 } else { 0x0380_FFF8 };
    nds(|n| n.set_ime(cpu, 1));
    if discard {
        let f = ld32(c, flags);
        st32(c, flags, f & !mask);
    }
    loop {
        let f = ld32(c, flags);
        if f & mask != 0 {
            st32(c, flags, f & !(f & mask));
            return;
        }
        sched::halt(c);
    }
}

fn isqrt64(v: u64) -> u32 {
    let mut r = (v as f64).sqrt() as u64;
    while r * r > v {
        r -= 1;
    }
    while (r + 1) * (r + 1) <= v {
        r += 1;
    }
    r as u32
}

fn cpu_set(c: &mut Cpu, fast: bool) {
    let (mut src, mut dst) = (c.r[0], c.r[1]);
    let mut n = c.r[2] & 0x1F_FFFF;
    let fill = c.r[2] >> 24 & 1 != 0;
    let words = fast || c.r[2] >> 26 & 1 != 0;
    if fast {
        n = (n + 7) & !7;
    }
    if words {
        src &= !3;
        dst &= !3;
        let mut v = ld32(c, src);
        for i in 0..n {
            if !fill {
                v = ld32(c, src + 4 * i);
            }
            st32(c, dst + 4 * i, v);
        }
    } else {
        src &= !1;
        dst &= !1;
        let mut v = ld16(c, src);
        for i in 0..n {
            if !fill {
                v = ld16(c, src + 2 * i);
            }
            st16(c, dst + 2 * i, v);
        }
    }
    sched::spend(c, 8 * n as u64);
}

/// Writes bytes to a destination one at a time, or (VRAM) halfwords.
struct Out {
    dst: u32,
    half: bool,
    pend: u32,
    odd: bool,
}

impl Out {
    fn put(&mut self, c: &mut Cpu, b: u8) {
        if !self.half {
            st8(c, self.dst, b as u32);
            self.dst += 1;
            return;
        }
        if !self.odd {
            self.pend = b as u32;
            self.odd = true;
            return;
        }
        st16(c, self.dst, self.pend | (b as u32) << 8);
        self.dst += 2;
        self.odd = false;
    }

    /// A byte already written, `disp` back.
    fn back(&self, c: &mut Cpu, disp: u32) -> u8 {
        let pos = (self.dst + if self.odd { 1 } else { 0 }).wrapping_sub(disp);
        if self.half && self.odd && pos == self.dst {
            return self.pend as u8;
        }
        ld8(c, pos) as u8
    }
}

fn lz77(c: &mut Cpu, half: bool) {
    let mut src = c.r[0];
    let size = ld32(c, src) >> 8;
    src += 4;
    let mut o = Out { dst: c.r[1], half, pend: 0, odd: false };
    let mut done = 0;
    while done < size {
        let mut flags = ld8(c, src) as u8;
        src += 1;
        let mut k = 0;
        while k < 8 && done < size {
            if flags & 0x80 != 0 {
                let b0 = ld8(c, src);
                let b1 = ld8(c, src + 1);
                src += 2;
                let disp = ((b0 & 0xF) << 8 | b1) + 1;
                let len = (b0 >> 4) + 3;
                let mut i = 0;
                while i < len && done < size {
                    let b = o.back(c, disp);
                    o.put(c, b);
                    i += 1;
                    done += 1;
                }
            } else {
                let b = ld8(c, src) as u8;
                src += 1;
                o.put(c, b);
                done += 1;
            }
            k += 1;
            flags <<= 1;
        }
    }
    sched::spend(c, 4 * size as u64);
}

fn rl(c: &mut Cpu, half: bool) {
    let mut src = c.r[0];
    let size = ld32(c, src) >> 8;
    src += 4;
    let mut o = Out { dst: c.r[1], half, pend: 0, odd: false };
    let mut done = 0;
    while done < size {
        let f = ld8(c, src);
        src += 1;
        if f & 0x80 != 0 {
            let b = ld8(c, src) as u8;
            src += 1;
            let mut i = 0;
            while i < (f & 0x7F) + 3 && done < size {
                o.put(c, b);
                i += 1;
                done += 1;
            }
        } else {
            let mut i = 0;
            while i < (f & 0x7F) + 1 && done < size {
                let b = ld8(c, src) as u8;
                src += 1;
                o.put(c, b);
                i += 1;
                done += 1;
            }
        }
    }
    sched::spend(c, 4 * size as u64);
}

fn crc16(c: &mut Cpu, mut crc: u32, addr: u32, len: u32) -> u16 {
    for i in 0..len {
        crc ^= ld8(c, addr + i);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xA001 } else { crc >> 1 };
        }
    }
    crc as u16
}

/// A SWI.
pub fn swi(c: &mut Cpu, num: u32) {
    let cpu = c.cpu;
    match num {
        0x03 => {
            // WaitByLoop
            let n = (c.r[0] as i32).max(0) as u64;
            sched::spend(c, 8 * n);
            c.r[0] = 0;
            return;
        }
        0x04 => return intr_wait(c, c.r[0] & 1 != 0, c.r[1]), // IntrWait
        0x05 => return intr_wait(c, true, 1),                  // VBlankIntrWait
        0x06 => return sched::halt(c),                         // Halt
        0x08 => {
            // SoundBias (ARM7)
            if cpu == 7 {
                let v = if c.r[0] != 0 { 0x200 } else { 0 };
                nds(|n| n.sound_write(0x0400_0504, v as u32, 2));
                return;
            }
        }
        0x09 => {
            // Div
            let (n, d) = (c.r[0] as i32, c.r[1] as i32);
            if d == 0 {
                fatal_msg(&format!("swi Div by zero (lr {:08X})", c.r[14]));
            }
            let (q, r) = if n == i32::MIN && d == -1 { (n, 0) } else { (n / d, n % d) };
            c.r[0] = q as u32;
            c.r[1] = r as u32;
            c.r[3] = q.unsigned_abs();
            return sched::spend(c, 80);
        }
        0x0B => return cpu_set(c, false), // CpuSet
        0x0C => return cpu_set(c, true),  // CpuFastSet
        0x0D => {
            // Sqrt
            c.r[0] = isqrt64(c.r[0] as u64);
            return sched::spend(c, 80);
        }
        0x0E => {
            // GetCRC16
            c.r[0] = crc16(c, c.r[0] & 0xFFFF, c.r[1], c.r[2]) as u32;
            return;
        }
        0x0F => {
            c.r[0] = 0; // IsDebugger
            return;
        }
        0x11 => return lz77(c, false), // LZ77 to WRAM (bytes)
        0x12 => {
            // the ARM7's LZ77 to VRAM; the ARM9's takes callbacks
            if cpu == 7 {
                return lz77(c, true);
            }
        }
        0x14 => return rl(c, false), // RL to WRAM
        0x15 => {
            if cpu == 7 {
                return rl(c, true);
            }
        }
        // The ARM7 BIOS's sound tables, worked out from their definitions; every value checked
        // against the BIOS's own tables (64, 768 and 724 of them)
        0x1A => {
            // GetSineTable: a quarter wave, 64 halfwords
            if cpu == 7 {
                c.r[0] = (((c.r[0] & 63) as f64 * std::f64::consts::PI / 128.0).sin() * 32767.0).round() as i64 as u32 & 0xFFFF;
                return;
            }
        }
        0x1B => {
            // GetPitchTable: 2^(i/768) - 1 in 16.16, 768 halfwords
            if cpu == 7 {
                c.r[0] = (((2f64).powf((c.r[0] % 768) as f64 / 768.0) - 1.0) * 65536.0).round() as i64 as u32 & 0xFFFF;
                return;
            }
        }
        0x1C => {
            // GetVolumeTable: 724 bytes, -72.3 dB to 0 in tenths, out of 128, times the divider
            // the SDK pairs with each range; as the channels read 127 as 128, 128 is kept as 127
            // and 127 as 126
            if cpu == 7 {
                let i = (c.r[0] % 724) as i32;
                let db = i - 723;
                let mul = if db < -240 { 16.0 } else if db < -120 { 4.0 } else if db < -60 { 2.0 } else { 1.0 };
                let v = (128.0 * 10f64.powf(db as f64 / 200.0) * mul).round() as i64;
                c.r[0] = (if v >= 128 { 127 } else if v == 127 { 126 } else { v }) as u32;
                return;
            }
        }
        _ => {}
    }
    fatal_msg(&format!(
        "ARM{}: swi {:02X} not done (r0 {:08X} r1 {:08X} r2 {:08X}, lr {:08X})",
        cpu, num, c.r[0], c.r[1], c.r[2], c.r[14]
    ));
}
