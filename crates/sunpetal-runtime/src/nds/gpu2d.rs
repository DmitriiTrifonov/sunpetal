//! The two 2D engines, in software, a line at a time.
//!
//! Engine A's registers are at 0x04000000, engine B's at 0x04001000; their palettes at 0x05000000
//! and 0x05000400 (backgrounds, then sprites), OAM at 0x07000000 and 0x07000400, their VRAM
//! through the bank areas. A line is drawn as it starts, with the registers as they are then (a
//! write in the HBlank before it counts, as on the DS).
//!
//! The layers: backgrounds in text, affine, extended (affine with 16-bit entries, 256-colour and
//! direct-colour bitmaps) and large-bitmap forms, with 16 or 256 colours and the extended
//! palettes; sprites (OBJ) in tiles, 1D and 2D mapping, 16 and 256 colours, affine and
//! double-size, bitmaps, semi-transparent and window sprites; windows 0, 1 and the OBJ window;
//! blending (alpha, brighter, darker); the master brightness; the display modes "off" (white),
//! graphics and VRAM display. On engine A, BG0 can be the 3D layer: its pixels are left to the
//! compositor, which has the 3D picture the rendering engine drew (`render3d`). Mosaic, display
//! capture and the main memory display are not drawn.
//!
//! Colours are kept as the DS keeps them inside, 6 bits a component: a palette's 5-bit component
//! c is 2c. A drawn pixel is two words (`Gpu2d::px`):
//!   word 0: the colour, RGB666 (r bits 0-5, g 6-11, b 12-17); bits 24-25 how it meets the 3D
//!           layer: 0 not at all, 1 the 3D is on top (bit 26: blended with this colour, the one
//!           under it, by the 3D's alpha), 2 this colour is on top, blended with the 3D under it
//!   word 1: EVA (bits 0-4) and EVB (8-12) of a blend with the 3D; EVY (16-20) and the 3D's
//!           brightness effect (21-22: 2 up, 3 down); the master brightness (24-25: 1 up, 2 down;
//!           26-30 its factor)
use super::video::{VRAM_BGA, VRAM_BGB, VRAM_BGEXTA, VRAM_BGEXTB, VRAM_LCDC, VRAM_OBJA, VRAM_OBJB, VRAM_OBJEXTA, VRAM_OBJEXTB};
use super::Nds;

pub(super) struct Gpu2d {
    /// [engine][line][x][2 words]
    pub(super) px: Vec<u32>,
    /// [engine][BG2, BG3][x, y]: the affine reference points.
    refs: [[[i32; 2]; 2]; 2],
    /// The engines draw (a headless run draws only for a picture's frame).
    pub(super) on: bool,
}

impl Gpu2d {
    pub(super) fn new() -> Gpu2d {
        Gpu2d { px: vec![0; 2 * 192 * 256 * 2], refs: [[[0; 2]; 2]; 2], on: true }
    }

    fn at(e: usize, y: usize) -> usize {
        (e * 192 + y) * 256 * 2
    }
}

fn sext28(v: u32) -> i32 {
    ((v << 4) as i32) >> 4
}

// ---- colours ---------------------------------------------------------------------------------
fn c666(c: u16) -> u32 {
    (c as u32 & 0x1F) << 1 | (c as u32 >> 5 & 0x1F) << 7 | (c as u32 >> 10 & 0x1F) << 13
}

fn blend(a: u32, b: u32, eva: u32, evb: u32) -> u32 {
    let mut out = 0;
    for s in [0, 6, 12] {
        let v = ((a >> s & 63) * eva + (b >> s & 63) * evb + 8) >> 4;
        out |= v.min(63) << s;
    }
    out
}

fn brighter(a: u32, evy: u32) -> u32 {
    let mut out = 0;
    for s in [0, 6, 12] {
        let c = a >> s & 63;
        out |= (c + ((63 - c) * evy >> 4)) << s;
    }
    out
}

fn darker(a: u32, evy: u32) -> u32 {
    let mut out = 0;
    for s in [0, 6, 12] {
        let c = a >> s & 63;
        out |= (c - ((c * evy + 7) >> 4)) << s;
    }
    out
}

// A layer's pixel: the colour (RGB666) and what it is.
const PX_OPAQUE: u32 = 1 << 31;
/// The 3D layer: its colour is the compositor's.
const PX_3D: u32 = 1 << 30;
/// A semi-transparent sprite.
const PX_SEMI: u32 = 1 << 29;
/// A bitmap sprite: its alpha in bits 24-27.
const PX_BMPA: u32 = 1 << 28;

struct Line {
    e: usize,
    io: u32,
    dispcnt: u32,
    abg: usize,
    aobj: usize,
    abgext: usize,
    aobjext: usize,
    /// The layers each pixel's window lets through (bits 0-5).
    win: [u8; 256],
    /// The sprites' line: pixel, `PX_OPAQUE` if any.
    obj: [u32; 256],
    objprio: [u8; 256],
    objwin: [u8; 256],
    top: [u32; 256],
    below: [u32; 256],
    /// 0-3 BG, 4 OBJ, 5 backdrop.
    toplayer: [u8; 256],
    belowlayer: [u8; 256],
}

impl Nds {
    fn g16(&self, a: u32) -> u16 {
        self.io_reg16(9, a)
    }
    fn g32(&self, a: u32) -> u32 {
        self.io_reg(9, a)
    }

    fn reload_refs(&mut self, e: usize) {
        let io = if e == 1 { 0x0400_1000 } else { 0x0400_0000 };
        for k in 0..2 {
            self.gpu.refs[e][k][0] = sext28(self.g32(io + 0x28 + 0x10 * k as u32));
            self.gpu.refs[e][k][1] = sext28(self.g32(io + 0x2C + 0x10 * k as u32));
        }
    }

    /// A write to the ARM9's registers 0x000-0x06F or 0x1000-0x106F.
    pub(super) fn gpu2d_written(&mut self, off: u32) {
        let e = if off >= 0x1000 { 1 } else { 0 };
        let r = off & 0xFFF;
        let base = if e == 1 { 0x0400_1000 } else { 0x0400_0000 };
        if (0x28..0x30).contains(&r) {
            self.gpu.refs[e][0][((r - 0x28) >> 2) as usize] = sext28(self.g32(base + (r & !3)));
        } else if (0x38..0x40).contains(&r) {
            self.gpu.refs[e][1][((r - 0x38) >> 2) as usize] = sext28(self.g32(base + (r & !3)));
        }
    }

    pub(super) fn gpu2d_vblank(&mut self) {
        self.reload_refs(0);
        self.reload_refs(1);
    }

    /// POWCNT1 bit 15: engine A on the upper screen.
    pub(super) fn gpu2d_top_is_a(&self) -> bool {
        self.io_reg16(9, 0x0400_0304) & 0x8000 != 0
    }

    fn pal16(&self, e: usize, i: u32) -> u16 {
        let o = (if e == 1 { 0x400 } else { 0 }) + (i & 0x3FF) as usize * 2;
        u16::from_le_bytes([self.palette[o], self.palette[o + 1]])
    }

    fn oam16(&self, e: usize, o: usize) -> u16 {
        let o = (if e == 1 { 0x400 } else { 0 }) + o;
        u16::from_le_bytes([self.oam[o], self.oam[o + 1]])
    }

    fn windows(&self, l: &mut Line, y: i32) {
        let d = l.dispcnt;
        if d & 0xE000 == 0 {
            l.win.fill(0x3F);
            return;
        }
        let winout = self.g16(l.io + 0x4A);
        let winin = self.g16(l.io + 0x48);
        l.win.fill((winout & 0x3F) as u8);
        if d & 0x8000 != 0 {
            for x in 0..256 {
                if l.objwin[x] != 0 {
                    l.win[x] = (winout >> 8 & 0x3F) as u8;
                }
            }
        }
        for w in (0..2).rev() {
            if d & (0x2000 << w) == 0 {
                continue;
            }
            let h = self.g16(l.io + 0x40 + 2 * w);
            let v = self.g16(l.io + 0x44 + 2 * w);
            let (x1, x2, y1, y2) = ((h >> 8) as i32, (h & 0xFF) as i32, (v >> 8) as i32, (v & 0xFF) as i32);
            let iny = if y1 <= y2 { y >= y1 && y < y2 } else { y >= y1 || y < y2 };
            if !iny {
                continue;
            }
            let m = (winin >> (8 * w) & 0x3F) as u8;
            for x in 0..256i32 {
                let inx = if x1 <= x2 { x >= x1 && x < x2 } else { x >= x1 || x < x2 };
                if inx {
                    l.win[x as usize] = m;
                }
            }
        }
    }

    /// A text background's line.
    fn bg_text(&self, l: &Line, bg: usize, y: i32, out: &mut [u32; 256]) {
        let cnt = self.g16(l.io + 0x08 + 2 * bg as u32);
        let hofs = self.g16(l.io + 0x10 + 4 * bg as u32) as u32 & 0x1FF;
        let vofs = self.g16(l.io + 0x12 + 4 * bg as u32) as u32 & 0x1FF;
        let size = cnt >> 14;
        let w: u32 = if size & 1 != 0 { 512 } else { 256 };
        let h: u32 = if size & 2 != 0 { 512 } else { 256 };
        let yy = (y as u32 + vofs) & (h - 1);
        let mut charbase = ((cnt as u32 >> 2) & 15) * 0x4000;
        let mut screenbase = ((cnt as u32 >> 8) & 31) * 0x800;
        if l.e == 0 {
            charbase += ((l.dispcnt >> 24) & 7) * 0x10000;
            screenbase += ((l.dispcnt >> 27) & 7) * 0x10000;
        }
        let c256 = cnt & 0x80 != 0;
        let ext = c256 && l.dispcnt & 0x4000_0000 != 0;
        let slot = bg as u32 + if bg < 2 && cnt & 0x2000 != 0 { 2 } else { 0 };
        let rowblock = (yy >> 8) * if w == 512 { 2 } else { 1 };
        for x in 0..256u32 {
            let xx = (x + hofs) & (w - 1);
            let se = screenbase + (rowblock + (xx >> 8)) * 0x800 + ((yy & 0xFF) >> 3) * 64 + ((xx & 0xFF) >> 3) * 2;
            let entry = self.vram_r16(l.abg, se) as u32;
            let tile = entry & 0x3FF;
            let px = (xx & 7) ^ if entry & 0x400 != 0 { 7 } else { 0 };
            let py = (yy & 7) ^ if entry & 0x800 != 0 { 7 } else { 0 };
            let pal = entry >> 12;
            if c256 {
                let idx = self.vram_r8(l.abg, charbase + tile * 64 + py * 8 + px) as u32;
                if idx == 0 {
                    out[x as usize] = 0;
                    continue;
                }
                let c = if ext { self.vram_r16(l.abgext, slot * 0x2000 + pal * 512 + idx * 2) } else { self.pal16(l.e, idx) };
                out[x as usize] = c666(c) | PX_OPAQUE;
            } else {
                let b = self.vram_r8(l.abg, charbase + tile * 32 + py * 4 + px / 2);
                let idx = if px & 1 != 0 { b >> 4 } else { b & 15 } as u32;
                if idx == 0 {
                    out[x as usize] = 0;
                    continue;
                }
                out[x as usize] = c666(self.pal16(l.e, pal * 16 + idx)) | PX_OPAQUE;
            }
        }
    }

    /// An affine background's line (the reference point already in place); kind: 0 affine (8-bit
    /// entries), 1 extended with 16-bit entries, 2 256-colour bitmap, 3 direct-colour bitmap, 4
    /// the large bitmap.
    fn bg_affine(&self, l: &Line, bg: usize, kind: u32, out: &mut [u32; 256]) {
        let cnt = self.g16(l.io + 0x08 + 2 * bg as u32);
        let pr = l.io + 0x20 + 0x10 * (bg as u32 - 2);
        let pa = self.g16(pr) as i16 as i32;
        let pc = self.g16(pr + 4) as i16 as i32;
        let (mut rx, mut ry) = (self.gpu.refs[l.e][bg - 2][0], self.gpu.refs[l.e][bg - 2][1]);
        let size = (cnt >> 14) as usize;
        let wrap = cnt & 0x2000 != 0;
        let (w, h): (u32, u32) = if kind <= 1 {
            let s = 128u32 << size;
            (s, s)
        } else if kind == 4 {
            if size & 1 != 0 { (1024, 512) } else { (512, 1024) }
        } else {
            const BW: [u32; 4] = [128, 256, 512, 512];
            const BH: [u32; 4] = [128, 256, 256, 512];
            (BW[size], BH[size])
        };
        let mut charbase = ((cnt as u32 >> 2) & 15) * 0x4000;
        let mut screenbase = ((cnt as u32 >> 8) & 31) * 0x800;
        if l.e == 0 && kind <= 1 {
            charbase += ((l.dispcnt >> 24) & 7) * 0x10000;
            screenbase += ((l.dispcnt >> 27) & 7) * 0x10000;
        }
        let bmpbase = ((cnt as u32 >> 8) & 31) * 0x4000;
        let ext = l.dispcnt & 0x4000_0000 != 0;
        for x in 0..256usize {
            let (mut tx, mut ty) = (rx >> 8, ry >> 8);
            rx = rx.wrapping_add(pa);
            ry = ry.wrapping_add(pc);
            if wrap {
                tx &= w as i32 - 1;
                ty &= h as i32 - 1;
            } else if tx < 0 || ty < 0 || tx >= w as i32 || ty >= h as i32 {
                out[x] = 0;
                continue;
            }
            let (ux, uy) = (tx as u32, ty as u32);
            out[x] = match kind {
                0 => {
                    let tile = self.vram_r8(l.abg, screenbase + (uy >> 3) * (w >> 3) + (ux >> 3)) as u32;
                    let idx = self.vram_r8(l.abg, charbase + tile * 64 + (uy & 7) * 8 + (ux & 7)) as u32;
                    if idx != 0 { c666(self.pal16(l.e, idx)) | PX_OPAQUE } else { 0 }
                }
                1 => {
                    let entry = self.vram_r16(l.abg, screenbase + ((uy >> 3) * (w >> 3) + (ux >> 3)) * 2) as u32;
                    let px = (ux & 7) ^ if entry & 0x400 != 0 { 7 } else { 0 };
                    let py = (uy & 7) ^ if entry & 0x800 != 0 { 7 } else { 0 };
                    let idx = self.vram_r8(l.abg, charbase + (entry & 0x3FF) * 64 + py * 8 + px) as u32;
                    if idx == 0 {
                        0
                    } else {
                        let c = if ext {
                            self.vram_r16(l.abgext, bg as u32 * 0x2000 + (entry >> 12) * 512 + idx * 2)
                        } else {
                            self.pal16(l.e, idx)
                        };
                        c666(c) | PX_OPAQUE
                    }
                }
                2 | 4 => {
                    let idx = self.vram_r8(l.abg, (if kind == 2 { bmpbase } else { 0 }) + uy * w + ux) as u32;
                    if idx != 0 { c666(self.pal16(l.e, idx)) | PX_OPAQUE } else { 0 }
                }
                _ => {
                    let c = self.vram_r16(l.abg, bmpbase + (uy * w + ux) * 2);
                    if c & 0x8000 != 0 { c666(c) | PX_OPAQUE } else { 0 }
                }
            };
        }
    }

    /// The sprites' line.
    fn sprites(&mut self, l: &mut Line, y: i32) {
        l.obj.fill(0);
        l.objprio.fill(4);
        l.objwin.fill(0);
        if l.dispcnt & 0x1000 == 0 {
            return;
        }
        const KW: [[i32; 4]; 3] = [[8, 16, 32, 64], [16, 32, 32, 64], [8, 8, 16, 32]];
        const KH: [[i32; 4]; 3] = [[8, 16, 32, 64], [8, 8, 16, 32], [16, 32, 32, 64]];
        let d = l.dispcnt;
        let ext = d & 0x8000_0000 != 0;
        for i in 0..128usize {
            let a0 = self.oam16(l.e, i * 8) as u32;
            let a1 = self.oam16(l.e, i * 8 + 2) as u32;
            let a2 = self.oam16(l.e, i * 8 + 4) as u32;
            let affine = a0 & 0x100 != 0;
            if !affine && a0 & 0x200 != 0 {
                continue;
            }
            let (shape, sz) = ((a0 >> 14) as usize, (a1 >> 14) as usize);
            if shape == 3 {
                continue;
            }
            let (w, h) = (KW[shape][sz], KH[shape][sz]);
            let (mut bw, mut bh) = (w, h);
            if affine && a0 & 0x200 != 0 {
                bw *= 2;
                bh *= 2;
            }
            let dy = (y - (a0 & 0xFF) as i32) & 0xFF;
            if dy >= bh {
                continue;
            }
            let mut x0 = (a1 & 0x1FF) as i32;
            if x0 >= 256 {
                x0 -= 512;
            }
            let mode = (a0 >> 10) & 3;
            if a0 & 0x1000 != 0 {
                self.once("2d", 1, || "2D: a mosaic sprite (mosaic is not drawn)".to_string());
            }
            let prio = ((a2 >> 10) & 3) as u8;
            let c256 = a0 & 0x2000 != 0;
            let tile = a2 & 0x3FF;
            let (mut pa, mut pb, mut pc, mut pd) = (0x100, 0, 0, 0x100);
            if affine {
                let g = (((a1 >> 9) & 31) * 32) as usize;
                pa = self.oam16(l.e, g + 6) as i16 as i32;
                pb = self.oam16(l.e, g + 14) as i16 as i32;
                pc = self.oam16(l.e, g + 22) as i16 as i32;
                pd = self.oam16(l.e, g + 30) as i16 as i32;
            }
            let alpha = a2 >> 12;
            if mode == 3 && alpha == 0 {
                continue;
            }
            for sx in 0..bw {
                let xs = x0 + sx;
                if !(0..=255).contains(&xs) {
                    continue;
                }
                let (tx, ty);
                if affine {
                    let (cx, cy) = (sx - bw / 2, dy - bh / 2);
                    tx = ((pa * cx + pb * cy) >> 8) + w / 2;
                    ty = ((pc * cx + pd * cy) >> 8) + h / 2;
                    if tx < 0 || ty < 0 || tx >= w || ty >= h {
                        continue;
                    }
                } else {
                    tx = if a1 & 0x1000 != 0 { w - 1 - sx } else { sx };
                    ty = if a1 & 0x2000 != 0 { h - 1 - dy } else { dy };
                }
                let (txu, tyu, wu) = (tx as u32, ty as u32, w as u32);
                let px;
                if mode == 3 {
                    let addr = if d & 0x40 != 0 {
                        tile * (128u32 << ((d >> 22) & 1)) + (tyu * wu + txu) * 2
                    } else {
                        let wide = d & 0x20 != 0;
                        let mask: u32 = if wide { 0x1F } else { 0x0F };
                        (tile & mask) * 0x10 + (tile & !mask) * 0x80 + tyu * if wide { 512 } else { 256 } + txu * 2
                    };
                    let c = self.vram_r16(l.aobj, addr);
                    if c & 0x8000 == 0 {
                        continue;
                    }
                    px = c666(c) | PX_OPAQUE | PX_BMPA | alpha << 24;
                } else {
                    let tbytes = if c256 { 64 } else { 32 };
                    let addr = if d & 0x10 != 0 {
                        tile * (32u32 << ((d >> 20) & 3)) + ((tyu >> 3) * (wu >> 3) + (txu >> 3)) * tbytes
                    } else {
                        tile * 32 + (tyu >> 3) * 1024 + (txu >> 3) * tbytes
                    };
                    let idx = if c256 {
                        self.vram_r8(l.aobj, addr + (tyu & 7) * 8 + (txu & 7)) as u32
                    } else {
                        let b = self.vram_r8(l.aobj, addr + (tyu & 7) * 4 + (txu & 7) / 2);
                        (if txu & 1 != 0 { b >> 4 } else { b & 15 }) as u32
                    };
                    if idx == 0 {
                        continue;
                    }
                    let c = if c256 {
                        if ext {
                            self.vram_r16(l.aobjext, (a2 >> 12) * 512 + idx * 2)
                        } else {
                            self.pal16(l.e, 0x100 + idx)
                        }
                    } else {
                        self.pal16(l.e, 0x100 + (a2 >> 12) * 16 + idx)
                    };
                    px = c666(c) | PX_OPAQUE | if mode == 1 { PX_SEMI } else { 0 };
                }
                let xu = xs as usize;
                if mode == 2 {
                    l.objwin[xu] = 1;
                    continue;
                }
                if prio < l.objprio[xu] {
                    l.obj[xu] = px;
                    l.objprio[xu] = prio;
                }
            }
        }
    }

    pub(super) fn gpu2d_line(&mut self, y: usize) {
        if !self.gpu.on {
            return;
        }
        if y == 0 {
            let cnt = self.g32(0x0400_0064);
            if cnt >> 31 != 0 {
                self.once("2d", 4, || "2D: display capture (DISPCAPCNT) is not done".to_string());
            }
        }
        self.draw_line(0, y);
        self.draw_line(1, y);
    }

    fn draw_line(&mut self, e: usize, y: usize) {
        let io = if e == 1 { 0x0400_1000 } else { 0x0400_0000 };
        let mut l = Box::new(Line {
            e,
            io,
            dispcnt: self.g32(io),
            abg: if e == 1 { VRAM_BGB } else { VRAM_BGA },
            aobj: if e == 1 { VRAM_OBJB } else { VRAM_OBJA },
            abgext: if e == 1 { VRAM_BGEXTB } else { VRAM_BGEXTA },
            aobjext: if e == 1 { VRAM_OBJEXTB } else { VRAM_OBJEXTA },
            win: [0; 256],
            obj: [0; 256],
            objprio: [4; 256],
            objwin: [0; 256],
            top: [0; 256],
            below: [0; 256],
            toplayer: [5; 256],
            belowlayer: [5; 256],
        });
        let mb = self.g16(io + 0x6C) as u32;
        let (mbmode, mbf) = ((mb >> 14) & 3, (mb & 0x1F).min(16));
        let w1base = if mbmode == 1 || mbmode == 2 { mbmode << 24 | mbf << 26 } else { 0 };
        let d = l.dispcnt;
        let dmode = (d >> 16) & if e == 1 { 1 } else { 3 };
        let pow = self.io_reg16(9, 0x0400_0304);
        let on = pow & if e == 1 { 0x200 } else { 0x2 } != 0;
        let base = Gpu2d::at(e, y);
        if !on || d & 0x80 != 0 {
            // the engine off or a forced blank: white
            for x in 0..256 {
                self.gpu.px[base + 2 * x] = 0x3FFFF;
                self.gpu.px[base + 2 * x + 1] = w1base;
            }
            return;
        }
        match dmode {
            1 => self.layers(&mut l, y as i32, base, w1base),
            0 => {
                for x in 0..256 {
                    self.gpu.px[base + 2 * x] = 0x3FFFF;
                    self.gpu.px[base + 2 * x + 1] = w1base;
                }
            }
            2 => {
                // VRAM display: a bank in the LCDC area, direct colour
                let vb = ((d >> 18) & 3) * 0x20000 + y as u32 * 512;
                for x in 0..256u32 {
                    self.gpu.px[base + 2 * x as usize] = c666(self.vram_r16(VRAM_LCDC, vb + 2 * x));
                    self.gpu.px[base + 2 * x as usize + 1] = w1base;
                }
            }
            _ => {
                self.once("2d", 3, || "2D: main memory display is not drawn".to_string());
                for x in 0..256 {
                    self.gpu.px[base + 2 * x] = 0;
                    self.gpu.px[base + 2 * x + 1] = w1base;
                }
            }
        }
    }

    /// An engine's graphics on line y: its BGs, OBJs (and engine A's 3D) through the windows and
    /// the blending, two words a pixel.
    fn layers(&mut self, l: &mut Line, y: i32, base: usize, w1base: u32) {
        let e = l.e;
        let d = l.dispcnt;
        self.sprites(l, y);
        self.windows(l, y);
        let backdrop = c666(self.pal16(e, 0));
        l.top.fill(backdrop);
        l.below.fill(backdrop);
        l.toplayer.fill(5);
        l.belowlayer.fill(5);
        let bgmode = (d & 7) as usize;
        let is3d = e == 0 && d & 8 != 0;
        // each BG's kind in this mode: -1 none, 0 text, 1 affine, 2 extended, 3 large bitmap
        const KIND: [[i8; 4]; 8] = [
            [0, 0, 0, 0],
            [0, 0, 0, 1],
            [0, 0, 1, 1],
            [0, 0, 0, 2],
            [0, 0, 1, 2],
            [0, 0, 2, 2],
            [0, -1, 3, -1],
            [-1, -1, -1, -1],
        ];
        let mut buf = [0u32; 256];
        fn push(l: &mut Line, x: usize, px: u32, layer: u8) {
            l.below[x] = l.top[x];
            l.belowlayer[x] = l.toplayer[x];
            l.top[x] = px;
            l.toplayer[x] = layer;
        }
        for prio in (0..4u8).rev() {
            for bg in (0..4usize).rev() {
                if d & (0x100 << bg) == 0 {
                    continue;
                }
                let cnt = self.g16(l.io + 0x08 + 2 * bg as u32);
                if (cnt & 3) as u8 != prio {
                    continue;
                }
                let kind = KIND[bgmode][bg];
                if kind < 0 {
                    continue;
                }
                if cnt & 0x40 != 0 {
                    self.once("2d", 2, || "2D: a mosaic background (mosaic is not drawn)".to_string());
                }
                if bg == 0 && is3d {
                    for x in 0..256 {
                        if l.win[x] & 1 != 0 {
                            push(l, x, PX_OPAQUE | PX_3D, 0);
                        }
                    }
                    continue;
                }
                match kind {
                    0 => self.bg_text(l, bg, y, &mut buf),
                    1 => self.bg_affine(l, bg, 0, &mut buf),
                    3 => self.bg_affine(l, bg, 4, &mut buf),
                    _ => {
                        let k = if cnt & 0x80 == 0 { 1 } else if cnt & 4 != 0 { 3 } else { 2 };
                        self.bg_affine(l, bg, k, &mut buf)
                    }
                }
                for x in 0..256 {
                    if buf[x] & PX_OPAQUE != 0 && l.win[x] & (1 << bg) != 0 {
                        push(l, x, buf[x], bg as u8);
                    }
                }
            }
            for x in 0..256 {
                if l.objprio[x] == prio && l.obj[x] & PX_OPAQUE != 0 && l.win[x] & 0x10 != 0 {
                    let px = l.obj[x];
                    push(l, x, px, 4);
                }
            }
        }
        // the affine reference points move on a line
        for k in 0..2 {
            let pr = l.io + 0x20 + 0x10 * k as u32;
            self.gpu.refs[e][k][0] += self.g16(pr + 2) as i16 as i32;
            self.gpu.refs[e][k][1] += self.g16(pr + 6) as i16 as i32;
        }

        // blending
        let bldcnt = self.g16(l.io + 0x50) as u32;
        let bldalpha = self.g16(l.io + 0x52) as u32;
        let eva = (bldalpha & 0x1F).min(16);
        let evb = ((bldalpha >> 8) & 0x1F).min(16);
        let evy = (self.g16(l.io + 0x54) as u32 & 0x1F).min(16);
        let bmode = (bldcnt >> 6) & 3;
        for x in 0..256 {
            let (t, b) = (l.top[x], l.below[x]);
            let (tl, bl) = (l.toplayer[x] as u32, l.belowlayer[x] as u32);
            let fx = l.win[x] & 0x20 != 0;
            let first = bldcnt & (1 << tl) != 0;
            let second = (bldcnt >> 8) & (1 << bl) != 0;
            let mut w1 = w1base;
            let w0;
            if t & PX_3D != 0 {
                // the 3D on top: the compositor blends it with what lies under it
                let mut v = (b & 0x3FFFF) | 1 << 24;
                if second && b & PX_3D == 0 {
                    v |= 1 << 26;
                }
                if fx && first && (bmode == 2 || bmode == 3) {
                    w1 |= evy << 16 | bmode << 21;
                }
                w0 = v;
            } else if fx && t & (PX_SEMI | PX_BMPA) != 0 && second {
                let (mut a, mut bb) = (eva, evb);
                if t & PX_BMPA != 0 {
                    a = ((t >> 24) & 15) + 1;
                    bb = 16 - a;
                }
                if b & PX_3D != 0 {
                    w0 = (t & 0x3FFFF) | 2 << 24;
                    w1 |= a | bb << 8;
                } else {
                    w0 = blend(t, b, a, bb);
                }
            } else if fx && first && bmode == 1 && second {
                if b & PX_3D != 0 {
                    w0 = (t & 0x3FFFF) | 2 << 24;
                    w1 |= eva | evb << 8;
                } else {
                    w0 = blend(t, b, eva, evb);
                }
            } else if fx && first && bmode == 2 {
                w0 = brighter(t, evy);
            } else if fx && first && bmode == 3 {
                w0 = darker(t, evy);
            } else {
                w0 = t & 0x3FFFF;
            }
            self.gpu.px[base + 2 * x] = w0;
            self.gpu.px[base + 2 * x + 1] = w1;
        }
    }

    /// Both screens as 256x384 RGB, the upper first.
    pub(crate) fn screens_rgb(&self) -> Vec<u8> {
        let mut rgb = vec![0u8; 256 * 384 * 3];
        let a_top = self.gpu2d_top_is_a();
        for s in 0..2 {
            let e = if (s == 0) == a_top { 0 } else { 1 };
            for y in 0..192 {
                for x in 0..256 {
                    let i = Gpu2d::at(e, y) + 2 * x;
                    let c = compose(&self.gpu.px[i..i + 2], if e == 0 { self.frame3d[y * 256 + x] } else { 0 });
                    let o = ((s * 192 + y) * 256 + x) * 3;
                    rgb[o] = (c >> 16) as u8;
                    rgb[o + 1] = (c >> 8) as u8;
                    rgb[o + 2] = c as u8;
                }
            }
        }
        rgb
    }
}

/// The compositor, in software: one pixel's words with the 3D's (RGB666 + alpha in bits 18-22),
/// to 0xRRGGBB.
pub(super) fn compose(w: &[u32], px3d: u32) -> u32 {
    let (w0, w1) = (w[0], w[1]);
    let mut c = w0 & 0x3FFFF;
    let mode = (w0 >> 24) & 3;
    let c3 = px3d & 0x3FFFF;
    let a3 = (px3d >> 18) & 31;
    if mode == 1 {
        if a3 == 0 {
            // a transparent 3D pixel: what lies under it shows
        } else if w0 & (1 << 26) != 0 {
            let eva = a3 + 1;
            let evb = 32 - eva;
            let mut out = 0;
            for s in [0, 6, 12] {
                out |= (((c3 >> s & 63) * eva + (c >> s & 63) * evb) >> 5) << s;
            }
            c = out;
        } else {
            c = c3;
        }
        if a3 != 0 {
            let (bm, evy) = ((w1 >> 21) & 3, (w1 >> 16) & 31);
            if bm == 2 {
                c = brighter(c, evy);
            } else if bm == 3 {
                c = darker(c, evy);
            }
        }
    } else if mode == 2 && a3 != 0 {
        c = blend(c, c3, w1 & 31, (w1 >> 8) & 31);
    }
    let (mm, mf) = ((w1 >> 24) & 3, (w1 >> 26) & 31);
    if mm == 1 {
        c = brighter(c, mf);
    } else if mm == 2 {
        c = darker(c, mf);
    }
    let (mut r, mut g, mut b) = (c & 63, c >> 6 & 63, c >> 12 & 63);
    r = r << 2 | r >> 4;
    g = g << 2 | g >> 4;
    b = b << 2 | b >> 4;
    r << 16 | g << 8 | b
}
