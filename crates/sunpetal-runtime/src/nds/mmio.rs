//! The address map outside the host arrays: the I/O registers of each CPU, the palettes, VRAM
//! through its banks, OAM, the GBA slot (empty), the BIOS areas, the ARM7's wireless block.
//!
//! Each CPU's registers 0x04000000-0x04001FFF are kept as plain storage; the ones with an effect
//! or a live value go to their device. Accesses are cut into halfwords (a word low half first)
//! except the registers that are words to the hardware: the IPC and card FIFOs, the geometry
//! engine's ports, the maths unit.
//!
use super::{ci, Nds};
use crate::machine::machine;

fn ones(size: usize) -> u32 {
    match size {
        4 => 0xFFFF_FFFF,
        2 => 0xFFFF,
        _ => 0xFF,
    }
}

fn le(p: &[u8], size: usize) -> u32 {
    let mut v = 0u32;
    for (i, b) in p.iter().take(size).enumerate() {
        v |= (*b as u32) << (8 * i);
    }
    v
}

fn put_le(p: &mut [u8], v: u32, size: usize) {
    for (i, b) in p.iter_mut().take(size).enumerate() {
        *b = (v >> (8 * i)) as u8;
    }
}

impl Nds {
    pub(crate) fn io_reg16(&self, cpu: i32, a: u32) -> u16 {
        let o = (a.wrapping_sub(0x0400_0000) & 0x1FFE) as usize;
        let io = &self.io[ci(cpu)];
        u16::from_le_bytes([io[o], io[o + 1]])
    }

    pub(crate) fn io_set16(&mut self, cpu: i32, a: u32, v: u16) {
        let o = (a.wrapping_sub(0x0400_0000) & 0x1FFE) as usize;
        self.io[ci(cpu)][o..o + 2].copy_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn io_reg(&self, cpu: i32, a: u32) -> u32 {
        self.io_reg16(cpu, a) as u32 | (self.io_reg16(cpu, a + 2) as u32) << 16
    }

    pub(crate) fn io_set(&mut self, cpu: i32, a: u32, v: u32) {
        self.io_set16(cpu, a, v as u16);
        self.io_set16(cpu, a + 2, (v >> 16) as u16);
    }

    pub(crate) fn mmio_reset(&mut self) {
        for io in &mut self.io {
            io.fill(0);
        }
        self.wifi.fill(0);
        // what melonDS's direct boot leaves
        self.io_set16(9, 0x0400_0304, 0x820F); // POWCNT1: both screens, both 2D engines, 3D
        self.io_set16(9, 0x0400_0300, 0x0001); // POSTFLG
        self.io_set16(7, 0x0400_0300, 0x0001);
        self.io_set16(7, 0x0400_0304, 0x0001); // POWCNT2: the speakers on
        self.io_set16(9, 0x0400_0246, 0x0300); // WRAMCNT = 3 (byte 0x247): all the shared WRAM to the ARM7
        self.io_set16(7, 0x0400_0240, 0x0300); // WRAMSTAT (byte 0x241)
        self.io_set16(7, 0x0400_0504, 0x0200); // SOUNDBIAS
        self.io_set16(7, 0x0400_0206, 0x0030); // the wireless wait states
    }

    // ---- I/O registers ------------------------------------------------------------------
    fn r16(&mut self, cpu: i32, a: u32) -> u16 {
        let off = a.wrapping_sub(0x0400_0000);
        let p = self.irq[ci(cpu)];
        match off {
            0x004 => return self.video_dispstat(cpu),
            0x006 => return self.video.vcount,
            0x100 | 0x104 | 0x108 | 0x10C => return self.timer_read(cpu, ((off - 0x100) / 4) as usize),
            0x130 => return self.keys,
            0x180 | 0x184 => return self.ipc_read(cpu, a) as u16,
            0x208 => return p.ime as u16,
            0x210 => return p.ie as u16,
            0x212 => return (p.ie >> 16) as u16,
            0x214 => return p.iflags as u16,
            0x216 => return (p.iflags >> 16) as u16,
            _ => {}
        }
        if (0x1A0..0x1C0).contains(&off) {
            return self.cart_read(a) as u16;
        }
        if cpu == 7 {
            match off {
                0x136 => return self.extkeys,
                0x138 => return self.rtc_read(),
                0x1C0 | 0x1C2 => return self.spi_read(a) as u16,
                _ => {}
            }
            if (0x400..0x520).contains(&off) {
                return self.sound_read(a, 2) as u16;
            }
        } else if (0x320..0x6A4).contains(&off) {
            return (self.gx_read(a & !3, 4) >> ((a & 2) * 8)) as u16;
        }
        self.io_reg16(cpu, a)
    }

    fn w16(&mut self, cpu: i32, a: u32, v: u16, mask: u16) {
        let off = a.wrapping_sub(0x0400_0000);
        let i = ci(cpu);
        // a byte write keeps the other byte as the device has it
        let old = if mask == 0xFFFF { 0 } else { self.r16(cpu, a) };
        let m = (old & !mask) | (v & mask);
        match off {
            0x004 => return self.video_dispstat_write(cpu, m),
            0x006 | 0x130 => return,
            0x180 | 0x184 => return self.ipc_write(cpu, a, m as u32, 2),
            0x208 => {
                self.irq[i].ime = (m & 1) as u32;
                self.io_set16(cpu, a, m);
                return self.irq_update(cpu);
            }
            0x210 => {
                self.irq[i].ie = (self.irq[i].ie & 0xFFFF_0000) | m as u32;
                return self.irq_update(cpu);
            }
            0x212 => {
                self.irq[i].ie = (self.irq[i].ie & 0xFFFF) | (m as u32) << 16;
                return self.irq_update(cpu);
            }
            0x214 => {
                self.irq[i].iflags &= !((v & mask) as u32);
                return self.irq_update(cpu);
            }
            0x216 => {
                self.irq[i].iflags &= !(((v & mask) as u32) << 16);
                return self.irq_update(cpu);
            }
            _ => {}
        }
        if (0x0B0..0x0E0).contains(&off) {
            // DMA: the control half starts it
            self.io_set16(cpu, a, m);
            if (off - 0x0B0) % 12 == 10 {
                self.dma_write(cpu, ((off - 0x0B0) / 12) as usize);
            }
            return;
        }
        if (0x100..0x110).contains(&off) {
            return self.timer_write(cpu, ((off - 0x100) / 4) as usize, ((off >> 1) & 1) as usize, m);
        }
        if (0x1A0..0x1C0).contains(&off) {
            return self.cart_write(cpu, a, m as u32, 2);
        }
        if cpu == 7 {
            match off {
                0x138 => return self.rtc_write(m),
                0x1C0 | 0x1C2 => return self.spi_write(a, m as u32),
                0x300 => {
                    self.io_set16(cpu, a, m);
                    if mask & 0xFF00 != 0 {
                        // HALTCNT: 0x80 halt; 0xC0 sleep (waits the same way). The wait itself
                        // happens when the write has returned to the CPU.
                        if (v >> 8) as u8 & 0x80 != 0 {
                            machine(|mm| mm.sched.halt_requested[i] = true);
                        }
                    }
                    return;
                }
                _ => {}
            }
            if (0x400..0x520).contains(&off) {
                return self.sound_write(a, m as u32, 2);
            }
        } else if (0x240..0x24A).contains(&off) {
            // VRAMCNT_A-G, WRAMCNT, VRAMCNT_H-I
            self.io_set16(cpu, a, m);
            for b in 0..2u32 {
                if (mask >> (8 * b)) & 0xFF == 0 {
                    continue;
                }
                let r = off + b;
                let x = (m >> (8 * b)) as u8;
                if r == 0x247 {
                    if x & 3 != 3 {
                        super::fatal(format!("WRAMCNT = {}: the runtime keeps the shared WRAM with the ARM7", x & 3));
                    }
                } else {
                    self.vram_cnt_write(if r < 0x247 { (r - 0x240) as usize } else { (r - 0x241) as usize }, x);
                }
            }
            return;
        } else if (0x320..0x6A4).contains(&off) {
            // the geometry engine's registers
            return self.gx_write(a, m as u32, 2);
        }
        self.io_set16(cpu, a, m);
        if cpu == 9 && off & !0x1000 < 0x70 {
            self.gpu2d_written(off);
        }
    }

    fn io_read(&mut self, cpu: i32, a: u32, size: usize) -> u32 {
        let off = a.wrapping_sub(0x0400_0000);
        if off == 0x10_0000 {
            return self.ipc_read(cpu, a);
        }
        if off == 0x10_0010 {
            return self.cart_data_word();
        }
        if cpu == 7 && off >= 0x80_0000 {
            let o = (a & 0xFFFF & !(size as u32 - 1)) as usize;
            return le(&self.wifi[o..], size);
        }
        if off >= 0x2000 {
            self.once("io read", a, || format!("ARM{}: {}-byte read of {:08X}, outside the I/O registers", cpu, size, a));
            return 0;
        }
        if cpu == 9 && (0x320..0x6A4).contains(&off) && size == 4 {
            return self.gx_read(a, 4);
        }
        if size == 4 {
            return self.r16(cpu, a) as u32 | (self.r16(cpu, a + 2) as u32) << 16;
        }
        let h = self.r16(cpu, a & !1);
        if size == 2 { h as u32 } else { (h as u32 >> ((a & 1) * 8)) & 0xFF }
    }

    fn io_write(&mut self, cpu: i32, a: u32, v: u32, size: usize) {
        let off = a.wrapping_sub(0x0400_0000);
        if cpu == 7 && off >= 0x80_0000 {
            let o = (a & 0xFFFF & !(size as u32 - 1)) as usize;
            put_le(&mut self.wifi[o..], v, size);
            return;
        }
        if off >= 0x2000 {
            self.once("io write", a, || {
                format!("ARM{}: {}-byte write of {:08X} to {:08X}, outside the I/O registers", cpu, size, v, a)
            });
            return;
        }
        if off == 0x188 {
            return self.ipc_write(cpu, a, v, size);
        }
        if (0x1A4..0x1B0).contains(&off) && size == 4 {
            return self.cart_write(cpu, a, v, 4);
        }
        if cpu == 9 {
            if (0x400..0x600).contains(&off) {
                return self.gx_write(a, v, size); // GXFIFO and the command ports
            }
            if (0x280..0x2C0).contains(&off) {
                // DIV and SQRT
                let w = a & !3;
                let o = (a & 3) * 8;
                let mk = if size == 4 { !0u32 } else { ((1u32 << (size * 8)) - 1) << o };
                let cur = self.io_reg(cpu, w);
                self.io_set(cpu, w, (cur & !mk) | (v << o & mk));
                return self.math_write(w);
            }
        }
        match size {
            4 => {
                self.w16(cpu, a, v as u16, 0xFFFF);
                self.w16(cpu, a + 2, (v >> 16) as u16, 0xFFFF);
            }
            2 => self.w16(cpu, a, v as u16, 0xFFFF),
            _ => {
                let sh = (a & 1) * 8;
                self.w16(cpu, a & !1, (v << sh) as u16, (0xFFu16) << sh);
            }
        }
    }

    // ---- the rest of the map ---------------------------------------------------------------
    pub(crate) fn mmio_read(&mut self, cpu: i32, a: u32, size: usize) -> u32 {
        match a >> 24 {
            0x04 => return self.io_read(cpu, a, size),
            0x05 if cpu == 9 => return le(&self.palette[(a & 0x7FF) as usize..], size),
            0x06 => return self.vram_read(cpu, a, size),
            0x07 if cpu == 9 => return le(&self.oam[(a & 0x7FF) as usize..], size),
            0x08 | 0x09 | 0x0A => return ones(size), // no GBA cartridge
            0x03 if cpu == 9 => return 0,            // the shared WRAM is all the ARM7's
            _ => {}
        }
        if cpu == 9 && a >= 0xFFFF_0000 {
            // the Nintendo logo, which the BIOS keeps at 0x20 as the cartridge's header does at 0xC0
            let o = a.wrapping_sub(0xFFFF_0020);
            if o as usize + size <= 0x9C {
                return le(&self.cart.rom[0xC0 + o as usize..], size);
            }
            self.once("bios", a, || format!("ARM9: read of the BIOS at {:08X}", a));
            return 0;
        }
        if cpu == 7 && a < 0x4000 {
            self.once("bios", a, || format!("ARM7: read of the BIOS at {:08X}", a));
            return 0;
        }
        self.once("unmapped read", a, || format!("ARM{}: {}-byte read of {:08X} (unmapped)", cpu, size, a));
        0
    }

    pub(crate) fn mmio_write(&mut self, cpu: i32, a: u32, v: u32, size: usize) {
        match a >> 24 {
            0x04 => return self.io_write(cpu, a, v, size),
            0x05 if cpu == 9 => {
                if size > 1 {
                    put_le(&mut self.palette[(a & 0x7FF) as usize..], v, size);
                }
                return;
            }
            0x06 => return self.vram_write(cpu, a, v, size),
            0x07 if cpu == 9 => {
                if size > 1 {
                    put_le(&mut self.oam[(a & 0x7FF) as usize..], v, size);
                }
                return;
            }
            0x08 | 0x09 | 0x0A => return,
            _ => {}
        }
        self.once("unmapped write", a, || format!("ARM{}: {}-byte write of {:08X} to {:08X} (unmapped)", cpu, size, v, a));
    }
}
