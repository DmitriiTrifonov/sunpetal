//! The display's timing, and VRAM with its banks.
//!
//! Lines: 263 of 355 dots, 6 system cycles a dot; HBlank from dot 256; VBlank from line 192 to 261
//! (the flag clears at 262). Each CPU has its DISPSTAT (its interrupt enables and VCount setting)
//! and gets its VBlank, HBlank and VCount interrupts and its VBlank and HBlank DMA. The 2D
//! engines and the geometry engine are not here yet: only their timing is.
//!
//! VRAM is nine banks (A-I, 656 KiB). VRAMCNT_A-I map each to the LCDC area, the 2D engines'
//! backgrounds and sprites, the 3D engine's texture and texture-palette slots, the extended
//! palettes or the ARM7. The CPUs reach the banks through a table of 16 KiB pages per area; where
//! banks overlap a read ORs them and a write goes to all.
use super::{ci, now, Ev, Nds, HBLANK_TICKS, LINE_TICKS};

pub(super) const VRAM_BGA: usize = 0;
pub(super) const VRAM_BGB: usize = 1;
pub(super) const VRAM_OBJA: usize = 2;
pub(super) const VRAM_OBJB: usize = 3;
pub(super) const VRAM_LCDC: usize = 4;
pub(super) const VRAM_ARM7: usize = 5;
pub(super) const VRAM_TEX: usize = 6;
pub(super) const VRAM_TEXPAL: usize = 7;
pub(super) const VRAM_BGEXTA: usize = 8;
pub(super) const VRAM_BGEXTB: usize = 9;
pub(super) const VRAM_OBJEXTA: usize = 10;
pub(super) const VRAM_OBJEXTB: usize = 11;
const NAREAS: usize = 12;
const PAGES: [usize; NAREAS] = [32, 8, 16, 8, 41, 16, 32, 6, 2, 2, 1, 1];

const BANK_OFF: [usize; 9] = [0x00000, 0x20000, 0x40000, 0x60000, 0x80000, 0x90000, 0x94000, 0x98000, 0xA0000];
const BANK_SIZE: [usize; 9] = [0x20000, 0x20000, 0x20000, 0x20000, 0x10000, 0x4000, 0x4000, 0x8000, 0x4000];
const VRAM_SIZE: usize = 0xA4000;
/// Where the DS starts drawing the next frame's 3D (melonDS's choice).
const RENDER_LINE: u16 = 215;

/// The banks' pages under a 16 KiB page of an area: offsets into the VRAM block.
#[derive(Clone, Copy, Default)]
struct Page {
    p: [usize; 4],
    n: usize,
}

pub(super) struct Video {
    pub(super) vcount: u16,
    /// The written bits of each CPU's DISPSTAT.
    dispstat: [u16; 2],
    /// VBlank (0), HBlank (1).
    flags: u16,
    line_start: u64,
    vram: Vec<u8>,
    cnt: [u8; 9],
    map: Vec<Vec<Page>>,
}

impl Video {
    pub(super) fn new() -> Video {
        Video {
            vcount: 0,
            dispstat: [0; 2],
            flags: 0,
            line_start: 0,
            vram: vec![0; VRAM_SIZE],
            cnt: [0; 9],
            map: vec![vec![Page::default(); 41]; NAREAS],
        }
    }

    fn map_bank(&mut self, area: usize, at: usize, bank: usize) {
        let mut o = 0;
        while o < BANK_SIZE[bank] {
            let pg = (at + o) >> 14;
            if pg >= PAGES[area] {
                return;
            }
            let page = &mut self.map[area][pg];
            if page.n < 4 {
                page.p[page.n] = BANK_OFF[bank] + o;
                page.n += 1;
            }
            o += 0x4000;
        }
    }

    /// Returns what the ARM7's VRAM status (WRAMSTAT's low bits) says.
    fn rebuild(&mut self) -> u8 {
        for a in &mut self.map {
            a.fill(Page::default());
        }
        let mut stat7 = 0u8;
        for b in 0..9usize {
            let v = self.cnt[b];
            if v & 0x80 == 0 {
                continue;
            }
            let mst = (v & if b <= 1 || b >= 7 { 3 } else { 7 }) as usize;
            let ofs = ((v >> 3) & 3) as usize;
            if mst == 0 {
                self.map_bank(VRAM_LCDC, BANK_OFF[b], b);
                continue;
            }
            match b {
                0 | 1 => match mst {
                    1 => self.map_bank(VRAM_BGA, 0x20000 * ofs, b),
                    2 => self.map_bank(VRAM_OBJA, 0x20000 * (ofs & 1), b),
                    _ => self.map_bank(VRAM_TEX, 0x20000 * ofs, b),
                },
                2 | 3 => match mst {
                    1 => self.map_bank(VRAM_BGA, 0x20000 * ofs, b),
                    2 => {
                        self.map_bank(VRAM_ARM7, 0x20000 * (ofs & 1), b);
                        stat7 |= 1 << (b - 2);
                    }
                    3 => self.map_bank(VRAM_TEX, 0x20000 * ofs, b),
                    4 => self.map_bank(if b == 2 { VRAM_BGB } else { VRAM_OBJB }, 0, b),
                    _ => {}
                },
                4 => match mst {
                    1 => self.map_bank(VRAM_BGA, 0, b),
                    2 => self.map_bank(VRAM_OBJA, 0, b),
                    3 => self.map_bank(VRAM_TEXPAL, 0, b),
                    4 => self.map_bank(VRAM_BGEXTA, 0, b), // slots 0-3 (its first 32 KiB)
                    _ => {}
                },
                5 | 6 => {
                    let at = 0x4000 * (ofs & 1) + 0x10000 * (ofs >> 1);
                    match mst {
                        1 => self.map_bank(VRAM_BGA, at, b),
                        2 => self.map_bank(VRAM_OBJA, at, b),
                        3 => self.map_bank(VRAM_TEXPAL, 0x4000 * ((ofs & 1) + (ofs >> 1) * 4), b),
                        4 => self.map_bank(VRAM_BGEXTA, 0x4000 * (ofs & 1), b), // slots 0-1 or 2-3
                        5 => self.map_bank(VRAM_OBJEXTA, 0, b),
                        _ => {}
                    }
                }
                7 => match mst {
                    1 => {
                        self.map_bank(VRAM_BGB, 0, b);
                        self.map_bank(VRAM_BGB, 0x10000, b);
                    }
                    2 => self.map_bank(VRAM_BGEXTB, 0, b),
                    _ => {}
                },
                _ => match mst {
                    1 => self.map_bank(VRAM_BGB, 0x8000, b),
                    2 => {
                        let mut o = 0;
                        while o < 0x20000 {
                            self.map_bank(VRAM_OBJB, o, b);
                            o += 0x4000;
                        }
                    }
                    3 => self.map_bank(VRAM_OBJEXTB, 0, b),
                    _ => {}
                },
            }
        }
        stat7
    }

    /// The `which`-th bank's byte under a VRAM address of `cpu`, as an offset into the block.
    fn page(&self, cpu: i32, a: u32, which: usize) -> Option<usize> {
        let (area, pg) = if cpu == 7 {
            (VRAM_ARM7, ((a & 0x3FFFF) >> 14) as usize)
        } else {
            match (a >> 21) & 7 {
                0 => (VRAM_BGA, ((a & 0x7FFFF) >> 14) as usize),
                1 => (VRAM_BGB, ((a & 0x1FFFF) >> 14) as usize),
                2 => (VRAM_OBJA, ((a & 0x3FFFF) >> 14) as usize),
                3 => (VRAM_OBJB, ((a & 0x1FFFF) >> 14) as usize),
                _ => {
                    let pg = ((a & 0xFFFFF) >> 14) as usize;
                    if pg >= 41 {
                        return None;
                    }
                    (VRAM_LCDC, pg)
                }
            }
        };
        let p = &self.map[area][pg];
        if which < p.n { Some(p.p[which] + (a & 0x3FFF) as usize) } else { None }
    }
}

impl Nds {
    // ---- timing -------------------------------------------------------------------------------
    fn vmatch(&self, cpu: i32) -> bool {
        let d = self.video.dispstat[ci(cpu)];
        self.video.vcount == ((d >> 8) | (d & 0x80) << 1)
    }

    pub(super) fn video_dispstat(&self, cpu: i32) -> u16 {
        let mut v = self.video.dispstat[ci(cpu)] | self.video.flags;
        // events land at a quantum's end: read the time
        if now() >= self.video.line_start + HBLANK_TICKS {
            v |= 2;
        }
        if self.vmatch(cpu) {
            v |= 4;
        }
        v
    }

    pub(super) fn video_dispstat_write(&mut self, cpu: i32, v: u16) {
        self.video.dispstat[ci(cpu)] = v & 0xFFB8;
    }

    pub(super) fn hblank(&mut self, when: u64) {
        self.video.flags |= 2;
        for cpu in [9, 7] {
            if self.video.dispstat[ci(cpu)] & 0x10 != 0 {
                self.irq_raise(cpu, 1);
            }
        }
        if self.video.vcount < 192 {
            self.dma_event(9, 2);
        }
        self.schedule(Ev::Hblank, when + LINE_TICKS);
    }

    pub(super) fn line(&mut self, when: u64) {
        self.video.line_start = when;
        self.video.vcount = (self.video.vcount + 1) % 263;
        self.video.flags &= !2;
        let v = self.video.vcount;
        if v < 192 {
            self.gpu2d_line(v as usize);
        } else if v == 192 {
            self.gpu2d_vblank();
            self.gx_vblank();
            self.video.flags |= 1;
            self.vblanks += 1;
            for cpu in [9, 7] {
                if self.video.dispstat[ci(cpu)] & 0x08 != 0 {
                    self.irq_raise(cpu, 0);
                }
                self.dma_event(cpu, 1);
            }
        } else if v == RENDER_LINE {
            self.gx_render_start();
        } else if v == 262 {
            self.video.flags &= !1;
        }
        for cpu in [9, 7] {
            if self.vmatch(cpu) && self.video.dispstat[ci(cpu)] & 0x20 != 0 {
                self.irq_raise(cpu, 2);
            }
        }
        self.gx_line();
        self.schedule(Ev::Line, when + LINE_TICKS);
    }

    pub(super) fn video_reset(&mut self) {
        self.video = Video::new();
        self.schedule(Ev::Hblank, HBLANK_TICKS);
        self.schedule(Ev::Line, LINE_TICKS);
        let stat7 = self.video.rebuild();
        self.set_vram_stat7(stat7);
        self.gpu = super::gpu2d::Gpu2d::new();
        self.gx = super::gx::Gx::new();
        self.frame3d = vec![0; 256 * 192];
        self.gpu2d_line(0);
    }

    // ---- VRAM ---------------------------------------------------------------------------------
    fn set_vram_stat7(&mut self, stat7: u8) {
        let cur = self.io_reg16(7, 0x0400_0240) & 0xFF00;
        self.io_set16(7, 0x0400_0240, stat7 as u16 | cur);
    }

    pub(super) fn vram_cnt_write(&mut self, bank: usize, v: u8) {
        if self.video.cnt[bank] == v {
            return;
        }
        self.video.cnt[bank] = v;
        self.trace(|| format!("VRAMCNT_{} = {:02X}", (b'A' + bank as u8) as char, v));
        let stat7 = self.video.rebuild();
        self.set_vram_stat7(stat7);
    }

    fn area_page(&self, area: usize, off: u32) -> &Page {
        let (pg, n) = ((off >> 14) as usize, PAGES[area]);
        &self.video.map[area][if pg < n { pg } else { pg % n }]
    }

    /// A byte of an area as its users (the 2D engines) see it: the banks under it ORed.
    pub(super) fn vram_r8(&self, area: usize, off: u32) -> u8 {
        let p = self.area_page(area, off);
        let o = (off & 0x3FFF) as usize;
        let mut v = 0;
        for k in 0..p.n {
            v |= self.video.vram[p.p[k] + o];
        }
        v
    }

    pub(super) fn vram_r16(&self, area: usize, off: u32) -> u16 {
        let p = self.area_page(area, off);
        let o = (off & 0x3FFE) as usize;
        let mut v = 0;
        for k in 0..p.n {
            let q = p.p[k] + o;
            v |= u16::from_le_bytes([self.video.vram[q], self.video.vram[q + 1]]);
        }
        v
    }

    /// An area's first `size` bytes as read (the banks under each page ORed; unmapped pages are 0).
    pub(super) fn vram_copy(&self, area: usize, size: usize) -> Vec<u8> {
        let mut out = vec![0u8; size];
        let mut o = 0;
        while o < size {
            let p = &self.video.map[area][o >> 14];
            let n = (size - o).min(0x4000);
            for k in 0..p.n {
                for i in 0..n {
                    out[o + i] |= self.video.vram[p.p[k] + i];
                }
            }
            o += 0x4000;
        }
        out
    }

    pub(super) fn vram_read(&mut self, cpu: i32, a: u32, size: usize) -> u32 {
        let mut v = 0u32;
        let mut k = 0;
        while let Some(o) = self.video.page(cpu, a, k) {
            for i in 0..size {
                v |= (self.video.vram[o + i] as u32) << (8 * i);
            }
            k += 1;
        }
        v
    }

    pub(super) fn vram_write(&mut self, cpu: i32, a: u32, v: u32, size: usize) {
        let mut any = false;
        let mut k = 0;
        while let Some(o) = self.video.page(cpu, a, k) {
            for i in 0..size {
                self.video.vram[o + i] = (v >> (8 * i)) as u8;
            }
            any = true;
            k += 1;
        }
        if !any {
            self.once("vram", a >> 14, || format!("ARM{}: write to unmapped VRAM {:08X}", cpu, a));
        }
    }
}
