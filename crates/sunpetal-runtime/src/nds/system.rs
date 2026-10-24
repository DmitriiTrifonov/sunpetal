//! Timers, DMA, IPC and the ARM9's maths unit.
//!
//! Timers count on the system clock (2 ticks) through their prescaler; their counters are worked
//! out from the time when read, and an event at the next overflow raises the interrupt and feeds
//! a count-up timer. DMA copies a whole transfer at once, when started (immediate, and the
//! geometry FIFO's, which never fills here), at VBlank or HBlank, or for each word the card has
//! ready. The IPC registers are the two CPUs' SYNC nibbles and two FIFOs of 16 words, with their
//! interrupts. The maths unit answers at once, with the results melonDS gives (division by zero
//! included).
use std::collections::VecDeque;

use super::{ci, now, Ev, Nds};

#[derive(Default, Clone, Copy)]
struct Timer {
    reload: u16,
    ctl: u16,
    counter: u16,
    base: u64,
}

const SHIFT: [u32; 4] = [0, 6, 8, 10];

fn period(t: &Timer) -> u64 {
    2u64 << SHIFT[(t.ctl & 3) as usize]
}

fn free_running(n: usize, t: &Timer) -> bool {
    t.ctl & 0x80 != 0 && !(n > 0 && t.ctl & 4 != 0)
}

#[derive(Default, Clone, Copy)]
struct Dma {
    src: u32,
    dst: u32,
    count: u32,
    on: bool,
}

pub(super) struct System {
    tm: [[Timer; 4]; 2],
    dma: [[Dma; 4]; 2],
    /// Each CPU's own bits: output nibble (8-11), IRQ enable (14).
    sync: [u16; 2],
    /// Each CPU's FIFO control: bits 2, 10, 14, 15.
    fcnt: [u16; 2],
    /// Each CPU's send FIFO (the other's receive FIFO).
    fifo: [VecDeque<u32>; 2],
    last: [u32; 2],
}

impl System {
    pub(super) fn new() -> System {
        System {
            tm: [[Timer::default(); 4]; 2],
            dma: [[Dma::default(); 4]; 2],
            sync: [0; 2],
            fcnt: [0; 2],
            fifo: [VecDeque::new(), VecDeque::new()],
            last: [0; 2],
        }
    }
}

fn other(cpu: i32) -> i32 {
    if cpu == 9 { 7 } else { 9 }
}

impl Nds {
    // ---- timers -----------------------------------------------------------------------------
    /// The counter moves on by `counts`; overflows raise the interrupt and feed the next timer.
    fn timer_advance(&mut self, cpu: i32, n: usize, counts: u64) {
        let t = &mut self.sys.tm[ci(cpu)][n];
        let v = t.counter as u64 + counts;
        if v < 0x10000 {
            t.counter = v as u16;
            return;
        }
        let rest = v - 0x10000;
        let span = 0x10000 - t.reload as u64;
        let ov = 1 + rest / span;
        t.counter = (t.reload as u64 + rest % span) as u16;
        let irq = t.ctl & 0x40 != 0;
        if irq {
            self.irq_raise(cpu, 3 + n as u32);
        }
        if n < 3 {
            let u = self.sys.tm[ci(cpu)][n + 1];
            if u.ctl & 0x84 == 0x84 {
                self.timer_advance(cpu, n + 1, ov);
            }
        }
    }

    fn timer_update(&mut self, cpu: i32, n: usize, now: u64) {
        let t = self.sys.tm[ci(cpu)][n];
        if !free_running(n, &t) || now <= t.base {
            return;
        }
        let p = period(&t);
        let counts = (now - t.base) / p;
        self.sys.tm[ci(cpu)][n].base += counts * p;
        if counts != 0 {
            self.timer_advance(cpu, n, counts);
        }
    }

    fn timer_schedule(&mut self, cpu: i32, n: usize) {
        let t = self.sys.tm[ci(cpu)][n];
        if !free_running(n, &t) {
            return self.cancel(Ev::Timer(cpu, n));
        }
        self.schedule(Ev::Timer(cpu, n), t.base + (0x10000 - t.counter as u64) * period(&t));
    }

    pub(super) fn timer_event(&mut self, cpu: i32, n: usize, when: u64) {
        self.timer_update(cpu, n, when);
        self.timer_schedule(cpu, n);
    }

    pub(super) fn timer_read(&mut self, cpu: i32, n: usize) -> u16 {
        self.timer_update(cpu, n, now());
        self.sys.tm[ci(cpu)][n].counter
    }

    pub(super) fn timer_write(&mut self, cpu: i32, n: usize, half: usize, v: u16) {
        let now = now();
        self.timer_update(cpu, n, now);
        self.io_set16(cpu, 0x0400_0100 + 4 * n as u32 + 2 * half as u32, v);
        let t = &mut self.sys.tm[ci(cpu)][n];
        if half == 0 {
            t.reload = v;
            return;
        }
        let was = t.ctl & 0x80 != 0;
        t.ctl = v & 0xC7;
        if !was && t.ctl & 0x80 != 0 {
            t.counter = t.reload;
            t.base = now;
        } else if free_running(n, t) {
            let p = period(t);
            t.base = t.base.max(now - (now - t.base) % p);
        }
        self.timer_schedule(cpu, n);
    }

    // ---- DMA --------------------------------------------------------------------------------
    fn dma_mode(cpu: i32, ctl: u16) -> u32 {
        if cpu == 9 { (ctl as u32 >> 11) & 7 } else { (ctl as u32 >> 12) & 3 }
    }

    fn dma_run(&mut self, cpu: i32, ch: usize) {
        let d = self.sys.dma[ci(cpu)][ch];
        let base = 0x0400_00B0 + 12 * ch as u32;
        let ctl = self.io_reg16(cpu, base + 10);
        let unit: u32 = if ctl & 0x400 != 0 { 4 } else { 2 };
        let (dctl, sctl) = ((ctl >> 5) & 3, (ctl >> 7) & 3);
        let step = |c: u16| -> u32 {
            match c {
                1 => unit.wrapping_neg(),
                2 => 0,
                _ => unit,
            }
        };
        let (ds, ss) = (step(dctl), step(sctl));
        let (mut s, mut t) = (d.src, d.dst);
        for _ in 0..d.count {
            if unit == 4 {
                let v = self.rd(cpu, s & !3, 4);
                self.wr(cpu, t & !3, v, 4);
            } else {
                let v = self.rd(cpu, s & !1, 2);
                self.wr(cpu, t & !1, v, 2);
            }
            s = s.wrapping_add(ss);
            t = t.wrapping_add(ds);
        }
        let dst_reload = self.io_reg(cpu, base + 4);
        let mode = Self::dma_mode(cpu, ctl);
        let d = &mut self.sys.dma[ci(cpu)][ch];
        d.src = s;
        d.dst = if dctl == 3 { dst_reload } else { t };
        if ctl & 0x200 != 0 && mode != 0 {
            // repeat: the count again (not the ARM7's top bits)
            let n = self.io_reg16(cpu, base + 8) as u32 | ((self.io_reg16(cpu, base + 10) & 0x1F) as u32) << 16;
            self.sys.dma[ci(cpu)][ch].count = n;
        } else {
            self.sys.dma[ci(cpu)][ch].on = false;
            self.io_set16(cpu, base + 10, ctl & 0x7FFF);
        }
        if ctl & 0x4000 != 0 {
            self.irq_raise(cpu, 8 + ch as u32);
        }
    }

    /// A DMA channel's control word changed.
    pub(super) fn dma_write(&mut self, cpu: i32, ch: usize) {
        let base = 0x0400_00B0 + 12 * ch as u32;
        let ctl = self.io_reg16(cpu, base + 10);
        if ctl & 0x8000 == 0 {
            self.sys.dma[ci(cpu)][ch].on = false;
            return;
        }
        if self.sys.dma[ci(cpu)][ch].on {
            return;
        }
        let amask = if cpu == 9 { 0x0FFF_FFFF } else { 0x07FF_FFFF };
        let src = self.io_reg(cpu, base) & amask;
        let dst = self.io_reg(cpu, base + 4) & amask;
        let mut n = self.io_reg16(cpu, base + 8) as u32;
        if cpu == 9 {
            n |= ((ctl & 0x1F) as u32) << 16;
        }
        let maxn: u32 = if cpu == 9 {
            0x20_0000
        } else if ch == 3 {
            0x1_0000
        } else {
            0x4000
        };
        n &= maxn - 1;
        let d = &mut self.sys.dma[ci(cpu)][ch];
        d.on = true;
        d.src = src;
        d.dst = dst;
        d.count = if n != 0 { n } else { maxn };
        let (count, mode) = (d.count, Self::dma_mode(cpu, ctl));
        self.trace(|| {
            format!("DMA{}.{}: {:08X} -> {:08X}, {} x {}, mode {}", cpu, ch, src, dst, count, if ctl & 0x400 != 0 { 4 } else { 2 }, mode)
        });
        if mode == 0 || (cpu == 9 && mode == 7) {
            self.dma_run(cpu, ch);
        } else if (cpu == 9 && mode == 5) || (cpu == 7 && mode == 2) {
            if self.cart_word_ready() {
                self.dma_card(cpu);
            }
        } else if !(mode == 1 || (cpu == 9 && mode == 2)) {
            self.once("dma mode", (cpu * 16) as u32 + mode, || format!("ARM{}: DMA start mode {} not done", cpu, mode));
        }
    }

    /// Start mode 1 VBlank, 2 HBlank (both CPUs' numbering).
    pub(super) fn dma_event(&mut self, cpu: i32, mode: u32) {
        for ch in 0..4 {
            let d = self.sys.dma[ci(cpu)][ch];
            if d.on && Self::dma_mode(cpu, self.io_reg16(cpu, 0x0400_00B0 + 12 * ch as u32 + 10)) == mode {
                self.dma_run(cpu, ch);
            }
        }
    }

    /// A word of the card is ready.
    pub(super) fn dma_card(&mut self, cpu: i32) {
        let mode = if cpu == 9 { 5 } else { 2 };
        for ch in 0..4 {
            let d = self.sys.dma[ci(cpu)][ch];
            if d.on && Self::dma_mode(cpu, self.io_reg16(cpu, 0x0400_00B0 + 12 * ch as u32 + 10)) == mode {
                while self.sys.dma[ci(cpu)][ch].on && self.cart_word_ready() {
                    self.dma_run(cpu, ch);
                }
                return;
            }
        }
    }

    // ---- IPC --------------------------------------------------------------------------------
    pub(super) fn ipc_read(&mut self, cpu: i32, addr: u32) -> u32 {
        let (me, you) = (ci(cpu), 1 - ci(cpu));
        let s = &mut self.sys;
        match addr & 0xFF_FFFF {
            0x180 => ((s.sync[you] >> 8 & 0xF) | (s.sync[me] & 0x4F00)) as u32,
            0x184 => {
                let mut v = (s.fcnt[me] & 0xC404) as u32;
                if s.fifo[me].is_empty() {
                    v |= 1;
                }
                if s.fifo[me].len() >= 16 {
                    v |= 2;
                }
                if s.fifo[you].is_empty() {
                    v |= 0x100;
                }
                if s.fifo[you].len() >= 16 {
                    v |= 0x200;
                }
                v
            }
            0x10_0000 => {
                if s.fcnt[me] & 0x8000 == 0 {
                    return s.last[me];
                }
                let Some(v) = s.fifo[you].pop_front() else {
                    s.fcnt[me] |= 0x4000;
                    return s.last[me];
                };
                s.last[me] = v;
                if s.fifo[you].is_empty() && s.fcnt[you] & 4 != 0 {
                    self.irq_raise(other(cpu), 17);
                }
                v
            }
            _ => 0,
        }
    }

    pub(super) fn ipc_write(&mut self, cpu: i32, addr: u32, v: u32, size: usize) {
        let (me, you) = (ci(cpu), 1 - ci(cpu));
        match addr & 0xFF_FFFF {
            0x180 => {
                self.sys.sync[me] = (v & 0x4F00) as u16;
                if v & 0x2000 != 0 && self.sys.sync[you] & 0x4000 != 0 {
                    self.irq_raise(other(cpu), 16);
                }
            }
            0x184 => {
                let mut old = self.sys.fcnt[me];
                if v & 8 != 0 {
                    self.sys.fifo[me].clear();
                }
                if v & 0x4000 != 0 {
                    old &= !0x4000;
                }
                self.sys.fcnt[me] = (old & 0x4000) | (v & 0x8404) as u16;
                if old & 4 == 0 && v & 4 != 0 && self.sys.fifo[me].is_empty() {
                    self.irq_raise(cpu, 17);
                }
                if old & 0x400 == 0 && v & 0x400 != 0 && !self.sys.fifo[you].is_empty() {
                    self.irq_raise(cpu, 18);
                }
            }
            0x188 => {
                if size != 4 {
                    self.once("ipc", size as u32, || format!("ARM{}: {}-byte write to IPCFIFOSEND", cpu, size));
                }
                if self.sys.fcnt[me] & 0x8000 == 0 {
                    return;
                }
                if self.sys.fifo[me].len() >= 16 {
                    self.sys.fcnt[me] |= 0x4000;
                    return;
                }
                self.sys.fifo[me].push_back(v);
                if self.sys.fifo[me].len() == 1 && self.sys.fcnt[you] & 0x400 != 0 {
                    self.irq_raise(other(cpu), 18);
                }
            }
            _ => {}
        }
    }

    // ---- the maths unit ---------------------------------------------------------------------
    fn rd64(&self, a: u32) -> u64 {
        self.io_reg(9, a) as u64 | (self.io_reg(9, a + 4) as u64) << 32
    }

    fn wr64(&mut self, a: u32, v: u64) {
        self.io_set(9, a, v as u32);
        self.io_set(9, a + 4, (v >> 32) as u32);
    }

    pub(super) fn math_write(&mut self, addr: u32) {
        let off = addr - 0x0400_0000;
        if (0x280..0x2B0).contains(&off) {
            let mut cnt = self.io_reg16(9, 0x0400_0280) as u32;
            let mode = cnt & 3;
            let num = self.rd64(0x0400_0290) as i64;
            let den = self.rd64(0x0400_0298) as i64;
            let (q, r): (i64, i64);
            if mode == 0 {
                let (n, d) = (num as i32, den as i32);
                if d == 0 {
                    q = (if n < 0 { 1u32 } else { u32::MAX }) as i64 | if n < 0 { 0xFFFF_FFFF_0000_0000u64 as i64 } else { 0 };
                    r = n as i64;
                } else if n == i32::MIN && d == -1 {
                    q = 0x8000_0000;
                    r = 0;
                } else {
                    q = (n / d) as i64;
                    r = (n % d) as i64;
                }
            } else {
                let d = if mode == 2 { den } else { den as i32 as i64 };
                if d == 0 {
                    q = if num < 0 { 1 } else { -1 };
                    r = num;
                } else if num == i64::MIN && d == -1 {
                    q = i64::MIN;
                    r = 0;
                } else {
                    q = num / d;
                    r = num % d;
                }
            }
            self.wr64(0x0400_02A0, q as u64);
            self.wr64(0x0400_02A8, r as u64);
            cnt = (cnt & 3) | if den == 0 { 0x4000 } else { 0 };
            self.io_set16(9, 0x0400_0280, cnt as u16);
            return;
        }
        if (0x2B0..0x2C0).contains(&off) {
            let cnt = self.io_reg16(9, 0x0400_02B0) & 1;
            let p = self.rd64(0x0400_02B8);
            let p = if cnt != 0 { p } else { p as u32 as u64 };
            self.io_set(9, 0x0400_02B4, isqrt(p) as u32);
            self.io_set16(9, 0x0400_02B0, cnt);
        }
    }
}

fn isqrt(mut v: u64) -> u64 {
    let (mut r, mut bit) = (0u64, 1u64 << 62);
    while bit > v {
        bit >>= 2;
    }
    while bit != 0 {
        if v >= r + bit {
            v -= r + bit;
            r = (r >> 1) + bit;
        } else {
            r >>= 1;
        }
        bit >>= 2;
    }
    r
}
