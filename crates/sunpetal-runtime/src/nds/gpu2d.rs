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
//! compositor, which has the 3D picture the rendering engine drew (`render3d`). Mosaic is drawn for
//! backgrounds (the first line of each block repeated, the first column of each block repeated)
//! and sprites; display capture and the main memory display are not.
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
    /// [engine][BG]: the line a mosaic block repeats.
    mosaic: Vec<[u32; 256]>,
    /// The dialogue line is drawn by the app in a crisp font: the game's raster text (BG3 of the
    /// upper screen, lines 144..191) is not drawn.
    pub(super) hide_dialogue: bool,
    /// The game's raster text in the band as the last frame drew it (BG3 of the upper screen, lines 144..191, also
    /// when it is hidden): a hash of its pixels, and how many are opaque.
    pub(super) band: (u64, u32),
    /// The box of the dark pixels of that text (x0, y0, x1, y1, inclusive), without the portrait's side: [0] the
    /// left 50 columns left out (the portrait is on the left), [1] the right 50 (it is on the right).
    pub(super) text_box: [Option<[u32; 4]>; 2],
    /// The 2D pack (8е.3): sprites whose pictures it holds are laid over the screen by `overlay_pack`.
    pub(super) pack: Option<std::sync::Arc<super::pack2d::Pack2d>>,
    /// [engine][OAM entry]: the sprite is replaced by the pack and not drawn.
    skip: [[bool; 128]; 2],
    /// This frame's replaced sprites.
    placed: Vec<Placed>,
    /// [engine][OAM entry]: the sprite last replaced there and for how many frames it has been kept after the pack lost it
    /// (a pressed icon of the case has other tiles and colours for a few frames, B-16).
    held: [Vec<Option<(Placed, u8)>>; 2],
    /// [engine][y][x]: the layer on top of the pixel (0-3 BG, 4 OBJ, 5 backdrop) as the last line drawn left it.
    top: Vec<u8>,
    /// This frame's replaced background layers: engine, layer, picture.
    layers_placed: Vec<(usize, usize, std::sync::Arc<super::pack2d::Img>)>,
}

/// Frames a replaced sprite keeps its picture after the pack has lost its key.
const HOLD_FRAMES: u8 = 8;

/// A sprite of the pack laid over the screen.
#[derive(Clone)]
struct Placed {
    e: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    flip_h: bool,
    flip_v: bool,
    img: std::sync::Arc<super::pack2d::Img>,
}

/// A sprite lifted out of the object memory (`Nds::sprite_bitmap`).
pub(super) struct Sprite {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub rgba: Vec<u8>,
    /// Hash of the tile data (the palette indices) and of the colours it uses.
    pub data: u64,
    pub pal: u64,
    pub flip_h: bool,
    pub flip_v: bool,
    pub affine: bool,
    /// OBJ mode: 0 normal, 1 semi-transparent, 2 window, 3 bitmap.
    pub mode: u8,
    pub prio: u8,
}

impl Sprite {
    /// `{w}x{h}_{data}_{pal}`, what names a sprite's picture in a dump and a 2D pack.
    pub fn key_name(&self) -> String {
        format!("{}x{}_{:016x}_{:016x}", self.w, self.h, self.data, self.pal)
    }
}

impl Gpu2d {
    pub(super) fn placed_count(&self) -> usize {
        self.placed.len() + self.layers_placed.len()
    }

    pub(super) fn new() -> Gpu2d {
        Gpu2d { px: vec![0; 2 * 192 * 256 * 2], refs: [[[0; 2]; 2]; 2], on: true, mosaic: vec![[0; 256]; 8], hide_dialogue: false, band: (0, 0), text_box: [None; 2], pack: None, skip: [[false; 128]; 2], placed: Vec::new(), held: [vec![None; 128], vec![None; 128]], top: vec![5; 2 * 192 * 256], layers_placed: Vec::new() }
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
        // a tile's map entry and its row of the character data are fetched once for its 8 pixels
        let mut x = 0u32;
        while x < 256 {
            let xx = (x + hofs) & (w - 1);
            let se = screenbase + (rowblock + (xx >> 8)) * 0x800 + ((yy & 0xFF) >> 3) * 64 + ((xx & 0xFF) >> 3) * 2;
            let entry = self.vram_r16(l.abg, se) as u32;
            let tile = entry & 0x3FF;
            let flip_x = if entry & 0x400 != 0 { 7 } else { 0 };
            let py = (yy & 7) ^ if entry & 0x800 != 0 { 7 } else { 0 };
            let pal = entry >> 12;
            // the pixels of this tile on the line: the first tile may be cut by the scroll
            let first = xx & 7;
            let n = (8 - first).min(256 - x);
            let o = &mut out[x as usize..(x + n) as usize];
            if c256 {
                let row = self.vram_bytes::<8>(l.abg, charbase + tile * 64 + py * 8);
                for (k, dst) in o.iter_mut().enumerate() {
                    let idx = row[((first + k as u32) ^ flip_x) as usize] as u32;
                    *dst = if idx == 0 {
                        0
                    } else {
                        let c = if ext { self.vram_r16(l.abgext, slot * 0x2000 + pal * 512 + idx * 2) } else { self.pal16(l.e, idx) };
                        c666(c) | PX_OPAQUE
                    };
                }
            } else {
                let row = self.vram_bytes::<4>(l.abg, charbase + tile * 32 + py * 4);
                // the 16 colours of the tile's palette (index 0 is transparent)
                let mut colours = [0u32; 16];
                for (i, c) in colours.iter_mut().enumerate().skip(1) {
                    *c = c666(self.pal16(l.e, pal * 16 + i as u32)) | PX_OPAQUE;
                }
                for (k, dst) in o.iter_mut().enumerate() {
                    let px = (first + k as u32) ^ flip_x;
                    let b = row[(px / 2) as usize];
                    *dst = colours[(if px & 1 != 0 { b >> 4 } else { b & 15 }) as usize];
                }
            }
            x += n;
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
            if self.gpu.skip[l.e][i] {
                continue;
            }
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
            let mosaic = a0 & 0x1000 != 0;
            let mreg = self.g16(l.io + 0x4C) as i32;
            let (moh, mov) = ((mreg >> 8 & 15) + 1, (mreg >> 12 & 15) + 1);
            let dy = if mosaic { (dy - y % mov).max(0) } else { dy };
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
                // a mosaic sprite takes each block's first column
                let sx = if mosaic { (xs - xs % moh - x0).clamp(0, bw - 1) } else { sx };
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
        if y == 0 {
            self.collect_replaced();
        }
        self.draw_line(0, y);
        self.draw_line(1, y);
    }

    fn new_line(&self, e: usize) -> Box<Line> {
        let io = if e == 1 { 0x0400_1000 } else { 0x0400_0000 };
        Box::new(Line {
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
        })
    }

    /// Writes every background layer of both engines, as it looks on its screen (scrolled, 256 x
    /// 192, transparent where empty), to `dir/bg{engine}{layer}_{hash}.png`. A debugging and
    /// picture-pack aid: the hash is of the pixels.
    pub(super) fn dump_backgrounds(&mut self, dir: &std::path::Path) -> usize {
        self.dump_backgrounds_rows(dir, &mut Vec::new())
    }

    /// `dump_backgrounds`, plus an `index.csv` row (`bg,engine,layer,0,0,256,192,key,file`) for each picture.
    fn dump_backgrounds_rows(&mut self, dir: &std::path::Path, rows: &mut Vec<String>) -> usize {
        let mut n = 0;
        for (e, bg, rgba, h) in self.bg_layers() {
            let name = format!("bg{}{}_{:016x}.png", e, bg, h);
            let _ = std::fs::create_dir_all(dir);
            if let Ok(f) = std::fs::File::create(dir.join(&name)) {
                let mut enc = png::Encoder::new(std::io::BufWriter::new(f), 256, 192);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                if let Ok(mut w) = enc.write_header() {
                    let _ = w.write_image_data(&rgba);
                    n += 1;
                    rows.push(format!("bg,{},{},0,0,256,192,{:016x},{}", e, bg, h, name));
                }
            }
        }
        n
    }

    /// Every background layer that shows, as it looks on its screen (scrolled, 256 x 192 RGBA, transparent where
    /// empty): (engine, layer, pixels, hash of the pixels).
    fn bg_layers(&mut self) -> Vec<(usize, usize, Vec<u8>, u64)> {
        let mut out = Vec::new();
        for e in 0..2 {
            let l = self.new_line(e);
            let d = l.dispcnt;
            let on = self.io_reg16(9, 0x0400_0304) & if e == 1 { 0x200 } else { 0x2 } != 0;
            if !on || d & 0x80 != 0 || (d >> 16) & if e == 1 { 1 } else { 3 } != 1 {
                continue;
            }
            let bgmode = (d & 7) as usize;
            const KIND: [[i8; 4]; 8] =
                [[0, 0, 0, 0], [0, 0, 0, 1], [0, 0, 1, 1], [0, 0, 0, 2], [0, 0, 1, 2], [0, 0, 2, 2], [0, -1, 3, -1], [-1, -1, -1, -1]];
            let saved = self.gpu.refs;
            for bg in 0..4usize {
                let kind = KIND[bgmode][bg];
                if d & (0x100 << bg) == 0 || kind < 0 || (bg == 0 && e == 0 && d & 8 != 0) {
                    continue;
                }
                let cnt = self.g16(l.io + 0x08 + 2 * bg as u32);
                self.gpu.refs = saved;
                let mut rgba = vec![0u8; 256 * 192 * 4];
                let mut buf = [0u32; 256];
                for y in 0..192usize {
                    match kind {
                        0 => self.bg_text(&l, bg, y as i32, &mut buf),
                        1 => self.bg_affine(&l, bg, 0, &mut buf),
                        3 => self.bg_affine(&l, bg, 4, &mut buf),
                        _ => {
                            let k = if cnt & 0x80 == 0 { 1 } else if cnt & 4 != 0 { 3 } else { 2 };
                            self.bg_affine(&l, bg, k, &mut buf)
                        }
                    }
                    for x in 0..256 {
                        if buf[x] & PX_OPAQUE != 0 {
                            let c = |sh: u32| {
                                let v = (buf[x] >> sh & 63) as u8;
                                v << 2 | v >> 4
                            };
                            rgba[(y * 256 + x) * 4..][..4].copy_from_slice(&[c(0), c(6), c(12), 255]);
                        }
                    }
                    if kind != 0 {
                        for k in 0..2 {
                            let pr = l.io + 0x20 + 0x10 * k as u32;
                            self.gpu.refs[e][k][0] += self.g16(pr + 2) as i16 as i32;
                            self.gpu.refs[e][k][1] += self.g16(pr + 6) as i16 as i32;
                        }
                    }
                }
                if rgba.chunks_exact(4).all(|p| p[3] == 0) {
                    continue;
                }
                let mut h = 0xcbf2_9ce4_8422_2325u64;
                for b in &rgba {
                    h = (h ^ *b as u64).wrapping_mul(0x100_0000_01b3);
                }
                out.push((e, bg, rgba, h));
            }
            self.gpu.refs = saved;
        }
        out
    }

    /// The sprite of OAM entry `i` of the engine of `l` as it lies in the object memory (no flip, no
    /// rotation), with the key of its data; `None` for one that is off, out of the picture, empty
    /// or of an unknown shape.
    fn sprite_bitmap(&self, l: &Line, i: usize) -> Option<Sprite> {
        let (e, d) = (l.e, l.dispcnt);
        let on = self.io_reg16(9, 0x0400_0304) & if e == 1 { 0x200 } else { 0x2 } != 0;
        if !on || d & 0x80 != 0 || d & 0x1000 == 0 || (d >> 16) & if e == 1 { 1 } else { 3 } != 1 {
            return None;
        }
        const KW: [[i32; 4]; 3] = [[8, 16, 32, 64], [16, 32, 32, 64], [8, 8, 16, 32]];
        const KH: [[i32; 4]; 3] = [[8, 16, 32, 64], [8, 8, 16, 32], [16, 32, 32, 64]];
        let ext = d & 0x8000_0000 != 0;
        let a0 = self.oam16(e, i * 8) as u32;
        let a1 = self.oam16(e, i * 8 + 2) as u32;
        let a2 = self.oam16(e, i * 8 + 4) as u32;
        let affine = a0 & 0x100 != 0;
        if (!affine && a0 & 0x200 != 0) || (a0 | a1 | a2) == 0 {
            return None;
        }
        let (shape, sz) = ((a0 >> 14) as usize, (a1 >> 14) as usize);
        if shape == 3 {
            return None;
        }
        let (w, h) = (KW[shape][sz], KH[shape][sz]);
        let (mut x0, y0) = ((a1 & 0x1FF) as i32, (a0 & 0xFF) as i32);
        if x0 >= 256 {
            x0 -= 512;
        }
        if x0 + w <= 0 || x0 >= 256 || (y0 + h <= 256 && y0 >= 192) {
            return None;
        }
        let bitmap = (a0 >> 10) & 3 == 3;
        let c256 = a0 & 0x2000 != 0;
        let tile = a2 & 0x3FF;
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        let (mut hd, mut hp) = (0xcbf2_9ce4_8422_2325u64, 0xcbf2_9ce4_8422_2325u64);
        let eat = |hh: &mut u64, v: u32| {
            for b in v.to_le_bytes() {
                *hh = (*hh ^ b as u64).wrapping_mul(0x100_0000_01b3);
            }
        };
        let mut used = [false; 256];
        for ty in 0..h as u32 {
            for tx in 0..w as u32 {
                let (wu, o) = (w as u32, ((ty * w as u32 + tx) * 4) as usize);
                let c;
                if bitmap {
                    let addr = if d & 0x40 != 0 {
                        tile * (128u32 << ((d >> 22) & 1)) + (ty * wu + tx) * 2
                    } else {
                        let mask: u32 = if d & 0x20 != 0 { 0x1F } else { 0x0F };
                        (tile & mask) * 0x10 + (tile & !mask) * 0x80 + ty * if d & 0x20 != 0 { 512 } else { 256 } + tx * 2
                    };
                    let v = self.vram_r16(l.aobj, addr);
                    eat(&mut hd, v as u32);
                    if v & 0x8000 == 0 {
                        continue;
                    }
                    c = v;
                } else {
                    let tbytes = if c256 { 64 } else { 32 };
                    let addr = if d & 0x10 != 0 {
                        tile * (32u32 << ((d >> 20) & 3)) + ((ty >> 3) * (wu >> 3) + (tx >> 3)) * tbytes
                    } else {
                        tile * 32 + (ty >> 3) * 1024 + (tx >> 3) * tbytes
                    };
                    let idx = if c256 {
                        self.vram_r8(l.aobj, addr + (ty & 7) * 8 + (tx & 7)) as u32
                    } else {
                        let b = self.vram_r8(l.aobj, addr + (ty & 7) * 4 + (tx & 7) / 2);
                        (if tx & 1 != 0 { b >> 4 } else { b & 15 }) as u32
                    };
                    eat(&mut hd, idx);
                    if idx == 0 {
                        continue;
                    }
                    used[idx as usize] = true;
                    c = if c256 {
                        if ext { self.vram_r16(l.aobjext, (a2 >> 12) * 512 + idx * 2) } else { self.pal16(e, 0x100 + idx) }
                    } else {
                        self.pal16(e, 0x100 + (a2 >> 12) * 16 + idx)
                    };
                }
                let c = c666(c);
                let ch = |sh: u32| {
                    let v = (c >> sh & 63) as u8;
                    v << 2 | v >> 4
                };
                rgba[o..o + 4].copy_from_slice(&[ch(0), ch(6), ch(12), 255]);
            }
        }
        // the palette hash covers the colours the sprite uses, in index order
        for (k, u) in used.iter().enumerate() {
            if *u {
                let c = if c256 {
                    if ext { self.vram_r16(l.aobjext, (a2 >> 12) * 512 + k as u32 * 2) } else { self.pal16(e, 0x100 + k as u32) }
                } else {
                    self.pal16(e, 0x100 + (a2 >> 12) * 16 + k as u32)
                };
                eat(&mut hp, k as u32);
                eat(&mut hp, c as u32);
            }
        }
        if rgba.chunks_exact(4).all(|p| p[3] == 0) {
            return None;
        }
        Some(Sprite { x: x0, y: y0, w, h, rgba, data: hd, pal: hp, flip_h: a1 & 0x1000 != 0, flip_v: a1 & 0x2000 != 0, affine, mode: ((a0 >> 10) & 3) as u8, prio: ((a2 >> 10) & 3) as u8 })
    }

    /// Every picture the 2D engines show, for a 2D pack (`docs/2D-UPSCALE.md`, 8е.2): the background
    /// layers as `bg{e}{layer}_{hash}.png` and each visible sprite as it lies in the object memory
    /// (no flip, no rotation) as `obj{e}_{w}x{h}_{data}_{pal}.png`, the key being the hashes of the
    /// sprite's tile data and of the palette it reads. `index.csv` (kind, engine, layer or OAM entry,
    /// x, y, w, h, key, file) is merged with what an earlier call wrote, sorted, so the same scenes
    /// give the same files. Returns the pictures written.
    pub(super) fn dump_2d(&mut self, dir: &std::path::Path) -> usize {
        let mut rows = Vec::new();
        let mut n = self.dump_backgrounds_rows(dir, &mut rows);
        for e in 0..2usize {
            let l = self.new_line(e);
            for i in 0..128usize {
                let Some(sp) = self.sprite_bitmap(&l, i) else { continue };
                let name = format!("obj{}_{}", e, sp.key_name());
                let name = format!("{}.png", name);
                let _ = std::fs::create_dir_all(dir);
                let path = dir.join(&name);
                if !path.exists() {
                    if let Ok(f) = std::fs::File::create(&path) {
                        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), sp.w as u32, sp.h as u32);
                        enc.set_color(png::ColorType::Rgba);
                        enc.set_depth(png::BitDepth::Eight);
                        if let Ok(mut wr) = enc.write_header() {
                            let _ = wr.write_image_data(&sp.rgba);
                            n += 1;
                        }
                    }
                }
                rows.push(format!("obj,{},{},{},{},{},{},{:016x}_{:016x},{}", e, i, sp.x, sp.y, sp.w, sp.h, sp.data, sp.pal, name));
            }
        }
        // merge with the index an earlier call wrote
        let idx = dir.join("index.csv");
        let mut all: std::collections::BTreeSet<String> = std::fs::read_to_string(&idx)
            .map(|t| t.lines().skip(1).map(str::to_owned).collect())
            .unwrap_or_default();
        all.extend(rows);
        let mut text = String::from("kind,engine,layer_or_oam,x,y,w,h,key,file\n");
        for r in &all {
            text.push_str(r);
            text.push('\n');
        }
        let _ = std::fs::write(idx, text);
        n
    }

    /// At a frame's first line: which sprites the pack holds a picture for (a plain sprite: not
    /// rotated, not blended, not a window, not a bitmap), so that they are not drawn and are laid
    /// over by `overlay_pack`.
    fn collect_replaced(&mut self) {
        self.gpu.skip = [[false; 128]; 2];
        self.gpu.placed.clear();
        self.gpu.layers_placed.clear();
        let Some(pack) = self.gpu.pack.clone() else { return };
        if pack.is_empty() {
            return;
        }
        // a blend or a fade over a layer would be lost under a picture laid over it: the engine draws those as they are
        let plain = |s: &Self, e: usize| {
            let io = if e == 1 { 0x0400_1000 } else { 0x0400_0000 };
            let (bldcnt, alpha, bldy, bright) = (s.g16(io + 0x50), s.g16(io + 0x52), s.g16(io + 0x54), s.g16(io + 0x6C));
            // a blend that changes nothing does not count: no first target, a brightness change of 0, or an alpha of 16:0
            // (the game sets that up on both screens of the device and the case menu)
            let blends = match bldcnt >> 6 & 3 {
                0 => false,
                1 => bldcnt & 0x3F != 0 && !(alpha & 31 >= 16 && alpha >> 8 & 31 == 0),
                _ => bldcnt & 0x3F != 0 && bldy & 31 != 0,
            };
            // the master brightness (a fade of the screen) does not count: `overlay_pack` fades the pictures the same way
            let _ = bright;
            !blends
        };
        // SUNPETAL_PACK_LOG=1: which engines draw as they are (a fade or a blend), when that changes (B-16)
        if std::env::var_os("SUNPETAL_PACK_LOG").is_some() {
            static LAST: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let io = |e: usize| if e == 1 { 0x0400_1000 } else { 0x0400_0000 };
            let now = (0..2usize).fold(0u32, |m, e| m | (!plain(self, e) as u32) << e);
            if LAST.swap(now, std::sync::atomic::Ordering::Relaxed) != now {
                eprintln!("pack2d: not plain A={} B={} (BLDCNT A {:04X} B {:04X}, BLDY A {} B {}, MASTER_BRIGHT A {:04X} B {:04X})", now & 1, now >> 1 & 1, self.g16(io(0) + 0x50), self.g16(io(1) + 0x50), self.g16(io(0) + 0x54), self.g16(io(1) + 0x54), self.g16(io(0) + 0x6C), self.g16(io(1) + 0x6C));
            }
        }
        if pack.has_layers() {
            for (e, bg, _, h) in self.bg_layers() {
                if !plain(self, e) {
                    continue;
                }
                if let Some(img) = pack.get_layer(h) {
                    self.gpu.layers_placed.push((e, bg, img));
                }
            }
        }
        for e in 0..2usize {
            let l = self.new_line(e);
            if !plain(self, e) {
                self.gpu.held[e].fill(None);
                continue;
            }
            for i in 0..128usize {
                let Some(sp) = self.sprite_bitmap(&l, i) else {
                    self.gpu.held[e][i] = None;
                    continue;
                };
                if sp.affine || sp.mode != 0 {
                    self.gpu.held[e][i] = None;
                    continue;
                }
                let Some(img) = pack.get(&(sp.w, sp.h, sp.data, sp.pal)) else {
                    // the same sprite in the same place and size, with other tiles: the picture it had stays a few frames
                    if let Some((p, age)) = self.gpu.held[e][i].take() {
                        if age < HOLD_FRAMES && (p.x, p.y, p.w, p.h) == (sp.x, sp.y, sp.w, sp.h) {
                            self.gpu.skip[e][i] = true;
                            self.gpu.placed.push(p.clone());
                            self.gpu.held[e][i] = Some((p, age + 1));
                        }
                    }
                    continue;
                };
                self.gpu.skip[e][i] = true;
                let p = Placed { e, x: sp.x, y: sp.y, w: sp.w, h: sp.h, flip_h: sp.flip_h, flip_v: sp.flip_v, img };
                self.gpu.held[e][i] = Some((p.clone(), 0));
                self.gpu.placed.push(p);
            }
        }
    }

    /// Lays the pack's pictures over `rgb` (the two screens, `CH` bytes a pixel, `s` times the DS's size).
    fn overlay_pack(&self, rgb: &mut [u8], s: usize, ch: usize, a_top: bool) {
        let (w, h3) = (256 * s, 192 * s);
        // the master brightness of each engine: the pictures fade with the screen (B-16: the old art showed through a fade)
        let fade = [0usize, 1].map(|e| {
            let mb = self.g16(if e == 1 { 0x0400_106C } else { 0x0400_006C }) as u32;
            ((mb >> 14) & 3, (mb & 0x1F).min(16))
        });
        let faded = |e: usize, c: &[u8]| -> [u32; 3] {
            let (m, f) = fade[e];
            let mut o = [c[0] as u32, c[1] as u32, c[2] as u32];
            for v in o.iter_mut() {
                *v = match m {
                    1 => *v + ((255 - *v) * f >> 4),
                    2 => *v - ((*v * f + 7) >> 4),
                    _ => *v,
                };
            }
            o
        };
        for (e, bg, img) in &self.gpu.layers_placed {
            let top = if (*e == 0) == a_top { 0 } else { h3 };
            let pic = img.scaled(w, h3);
            for y in 0..h3 {
                let mask = &self.gpu.top[(e * 192 + y / s) * 256..][..256];
                for x in 0..w {
                    if mask[x / s] != *bg as u8 {
                        continue;
                    }
                    let c = &pic[(y * w + x) * 4..][..4];
                    if c[3] == 0 {
                        continue;
                    }
                    let o = ((top + y) * w + x) * ch;
                    let f = faded(*e, c);
                    if c[3] == 255 {
                        for k in 0..3 {
                            rgb[o + k] = f[k] as u8;
                        }
                        continue;
                    }
                    let a = c[3] as u32;
                    for k in 0..3 {
                        rgb[o + k] = ((f[k] * a + rgb[o + k] as u32 * (255 - a) + 127) / 255) as u8;
                    }
                }
            }
        }
        for p in &self.gpu.placed {
            let top = if (p.e == 0) == a_top { 0 } else { h3 };
            let (pw, ph) = ((p.w as usize) * s, (p.h as usize) * s);
            for dy in 0..ph {
                let sy = (p.y * s as i32 + dy as i32) & (256 * s as i32 - 1);
                if sy as usize >= h3 {
                    continue;
                }
                for dx in 0..pw {
                    let sx = p.x * s as i32 + dx as i32;
                    if sx < 0 || sx as usize >= w {
                        continue;
                    }
                    let (fx, fy) = ((dx as f32 + 0.5) / pw as f32, (dy as f32 + 0.5) / ph as f32);
                    let (fx, fy) = (if p.flip_h { 1.0 - fx } else { fx }, if p.flip_v { 1.0 - fy } else { fy });
                    let c = p.img.sample(fx, fy);
                    if c[3] == 0 {
                        continue;
                    }
                    let o = ((top + sy as usize) * w + sx as usize) * ch;
                    let a = c[3] as u32;
                    let f = faded(p.e, &c);
                    for k in 0..3 {
                        rgb[o + k] = ((f[k] * a + rgb[o + k] as u32 * (255 - a) + 127) / 255) as u8;
                    }
                }
            }
        }
    }

    fn draw_line(&mut self, e: usize, y: usize) {
        let io = if e == 1 { 0x0400_1000 } else { 0x0400_0000 };
        let mut l = self.new_line(e);
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
                let mosaic = cnt & 0x40 != 0;
                let mreg = self.g16(l.io + 0x4C) as i32;
                let (mbh, mbv) = ((mreg & 15) as usize + 1, (mreg >> 4 & 15) as usize + 1);
                if bg == 0 && is3d {
                    for x in 0..256 {
                        if l.win[x] & 1 != 0 {
                            push(l, x, PX_OPAQUE | PX_3D, 0);
                        }
                    }
                    continue;
                }
                let cached = mosaic && y as usize % mbv != 0;
                if cached {
                    buf = self.gpu.mosaic[e * 4 + bg];
                } else {
                    match kind {
                        0 => self.bg_text(l, bg, y, &mut buf),
                        1 => self.bg_affine(l, bg, 0, &mut buf),
                        3 => self.bg_affine(l, bg, 4, &mut buf),
                        _ => {
                            let k = if cnt & 0x80 == 0 { 1 } else if cnt & 4 != 0 { 3 } else { 2 };
                            self.bg_affine(l, bg, k, &mut buf)
                        }
                    }
                    if mosaic {
                        for x in 0..256 {
                            buf[x] = buf[x - x % mbh];
                        }
                        self.gpu.mosaic[e * 4 + bg] = buf;
                    }
                }
                if e == 0 && bg == 3 && y >= 144 {
                    if y == 144 {
                        self.gpu.band = (0xcbf2_9ce4_8422_2325, 0);
                        self.gpu.text_box = [None; 2];
                    }
                    for x in 0..256 {
                        if buf[x] & PX_OPAQUE != 0 {
                            self.gpu.band.0 = (self.gpu.band.0 ^ (buf[x] as u64 | (x as u64) << 32 | (y as u64) << 48)).wrapping_mul(0x100_0000_01b3);
                            self.gpu.band.1 += 1;
                            // the game's text is near black; the panel and the portrait are not (the portrait's side is left out)
                            if buf[x] & 63 < 16 && buf[x] >> 6 & 63 < 16 && buf[x] >> 12 & 63 < 16 {
                                for (k, skip) in [(0usize, x < 50), (1, x >= 206)] {
                                    if !skip {
                                        let (x, y) = (x as u32, y as u32);
                                        let b = self.gpu.text_box[k].get_or_insert([x, y, x, y]);
                                        *b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
                                    }
                                }
                            }
                        }
                    }
                    if self.gpu.hide_dialogue {
                        continue;
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
            self.gpu.top[(e * 192 + y as usize) * 256 + x] = tl as u8;
        }
    }

    /// Both screens as RGB at `scale` (256 x scale by 384 x scale), the upper first: the 2D
    /// layers repeated per pixel, the 3D picture at its own scale (the nearest pixel).
    pub(crate) fn screens_rgb(&self, scale: usize) -> Vec<u8> {
        self.screens_pixels::<3>(scale)
    }

    /// As `screens_rgb`, four bytes a pixel, opaque.
    pub(crate) fn screens_rgba(&self, scale: usize) -> Vec<u8> {
        self.screens_pixels::<4>(scale)
    }

    /// The screens with `CH` bytes a pixel: red, green, blue and, for 4, an alpha of 255.
    fn screens_pixels<const CH: usize>(&self, scale: usize) -> Vec<u8> {
        let s = scale.max(1);
        let (w, h3) = (256 * s, 192 * s);
        let mut rgb = vec![255u8; w * 2 * h3 * CH];
        let a_top = self.gpu2d_top_is_a();
        let f3 = self.frame3d_scale.max(1);
        // the 3D picture may be wider than the screen: the middle of it is the screen
        let e3 = self.frame3d_wide;
        let w3 = (256 + 2 * e3) * f3;
        let c3 = e3 * f3;
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
        // rows of the picture, split across threads
        let rows_per = (2 * h3).div_ceil(threads);
        std::thread::scope(|sc| {
            for (k, chunk) in rgb.chunks_mut(rows_per * w * CH).enumerate() {
                sc.spawn(move || {
                    for (r, row) in chunk.chunks_mut(w * CH).enumerate() {
                        let ry = k * rows_per + r;
                        let (screen, y) = (ry / h3, ry % h3);
                        let e = if (screen == 0) == a_top { 0 } else { 1 };
                        let (y2, y3) = (y / s, y * f3 / s);
                        for x in 0..w {
                            let i = Gpu2d::at(e, y2) + 2 * (x / s);
                            let p3 = if e == 0 { self.frame3d[y3 * w3 + c3 + x * f3 / s] } else { 0 };
                            let c = compose(&self.gpu.px[i..i + 2], p3);
                            row[CH * x] = (c >> 16) as u8;
                            row[CH * x + 1] = (c >> 8) as u8;
                            row[CH * x + 2] = c as u8;
                        }
                    }
                });
            }
        });
        self.overlay_pack(&mut rgb, s, CH, a_top);
        rgb
    }
}

impl Nds {
    /// The upper screen alone, wider than the DS's own while the 3D is on it: `(each side, RGB at
    /// scale)`, (256 + 2 * each side) x scale by 192 x scale. The 2D layers are as they are in the
    /// middle and their edge columns run on to the sides (a rule, a panel); the 3D is the widened
    /// picture. `None` when the 3D is not on the upper screen or the view is not widened.
    pub(crate) fn upper_wide_rgb(&self, scale: usize) -> Option<(usize, Vec<u8>)> {
        self.upper_wide_rgb_layers(scale, 3)
    }

    /// As `upper_wide_rgb`, with only the layers in `layers` (bit 0 the 2D, bit 1 the 3D): for looking at a picture's parts.
    pub(crate) fn upper_wide_rgb_layers(&self, scale: usize, layers: u8) -> Option<(usize, Vec<u8>)> {
        let e3 = self.frame3d_wide;
        let a_top = self.gpu2d_top_is_a();
        if e3 == 0 || !a_top || self.io_reg16(9, 0x0400_0000) & 8 == 0 {
            return None;
        }
        let s = scale.max(1);
        let (f3, w3) = (self.frame3d_scale.max(1), (256 + 2 * e3) * self.frame3d_scale.max(1));
        let c3 = e3 * f3;
        let (w, h) = ((256 + 2 * e3) * s, 192 * s);
        let mut rgb = vec![0u8; w * h * 3];
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
        let rows_per = h.div_ceil(threads);
        std::thread::scope(|sc| {
            for (k, chunk) in rgb.chunks_mut(rows_per * w * 3).enumerate() {
                sc.spawn(move || {
                    for (r, row) in chunk.chunks_mut(w * 3).enumerate() {
                        let y = k * rows_per + r;
                        let (y2, y3) = (y / s, y * f3 / s);
                        for x in 0..w {
                            // the 2D layers at the nearest column of the screen's own 256 (a panel, a rule, a logo's ground run
                            // on; the 3D's far scenery is what has a drawing at its edge, `render3d::extend_sides`)
                            let x2 = (x / s).saturating_sub(e3).min(255);
                            let i = Gpu2d::at(0, y2) + 2 * x2;
                            // beside the DS's own picture the 3D shows only on the rows where the picture's
                            // edge has it: a panel or a rule over the 3D there runs on, not cut off by it
                            let x3 = x * f3 / s;
                            let edge = if x3 < c3 { c3 } else if x3 >= c3 + 256 * f3 { c3 + 256 * f3 - 1 } else { x3 };
                            let p3 = if (self.frame3d[y3 * w3 + edge] >> 18) & 31 == 0 { 0 } else { self.frame3d[y3 * w3 + x3] };
                            let words = if layers & 1 != 0 { [self.gpu.px[i], self.gpu.px[i + 1]] } else if layers & 2 != 0 { [1 << 24, 0] } else { [0, 0] }; // mode 1: the 3D replaces
                            let c = compose(&words, if layers & 2 != 0 { p3 } else { 0 });
                            row[3 * x] = (c >> 16) as u8;
                            row[3 * x + 1] = (c >> 8) as u8;
                            row[3 * x + 2] = c as u8;
                        }
                    }
                });
            }
        });
        Some((e3, rgb))
    }
}

/// The compositor, in software: one pixel's words with the 3D's (RGB666 + alpha in bits 18-22),
/// to 0xRRGGBB.
pub(super) fn compose(w: &[u32], px3d: u32) -> u32 {
    let (w0, w1) = (w[0], w[1]);
    let mut c = w0 & 0x3FFFF;
    let mode = (w0 >> 24) & 3;
    let mut c3 = px3d & 0x3FFFF;
    // the outline and the shading (`render3d::outline_pass`, `shade_pass`): the 3D's colour darkened,
    // where the 3D is on show
    let (line, shade) = ((px3d >> 24) & 15, (px3d >> 28) & 15);
    if line | shade != 0 {
        let dark = (85 * line).max(65 * shade);
        let f = |v: u32| v * (1500 - dark) / 1500;
        c3 = f(c3 & 63) | f(c3 >> 6 & 63) << 6 | f(c3 >> 12 & 63) << 12;
    }
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
