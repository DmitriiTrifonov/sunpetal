//! The native glyph blit of the dialogue window (K-189, `docs/NATIVE-DIALOG-DRAW.md`): what the game's font library
//! (Nitro `NNS_G2d` char canvas) does to draw one letter into tile data, as host code. It is the recompiled
//! `0x020338E0` (draw a glyph into the canvas, clipped to it), `0x020336A0` (one 8x8 tile of it) and `0x02034C5C`
//! (read bits of the glyph bitmap), word for word. The layouts are the library's (offsets named below), not the
//! game's numbers. Behind `SUNPETAL_NATIVE_DIALOG_DRAW=1`.
use crate::abi::Cpu;
use crate::services::{mem_read32, mem_read8, mem_write32};
use std::collections::HashMap;

/// Guest memory as the blit sees it.
pub trait Mem {
    fn r8(&mut self, a: u32) -> u32;
    fn r32(&mut self, a: u32) -> u32;
    fn w32(&mut self, a: u32, v: u32);
}

// canvas: `[0]` pixel data, `[4]`/`[8]` width/height in tiles, `[0xC]` bits per pixel, `[0x10]` tiles per row
const CV_DATA: u32 = 0;
const CV_W: u32 = 4;
const CV_H: u32 = 8;
const CV_BPP: u32 = 0xC;
const CV_STRIDE: u32 = 0x10;
// font object `[0]` -> font `[8]` -> resource: `[0]` cell width, `[1]` cell height, `[6]` bits per glyph pixel
const FONT_RES: u32 = 8;
const RES_CELL_W: u32 = 0;
const RES_CELL_H: u32 = 1;
const RES_BPP: u32 = 6;
// glyph: `[0]` -> width info (`[1]` glyph width), `[4]` bitmap
const GL_INFO: u32 = 0;
const GL_BITS: u32 = 4;

/// The bits of the glyph bitmap, most significant first (`0x02034C5C`): a pointer, the bits left in the byte, the byte.
struct Bits {
    ptr: u32,
    avail: i32,
    cur: u32,
}

impl Bits {
    fn read(&mut self, m: &mut impl Mem, n: i32) -> u32 {
        let r = if self.avail >= n {
            self.avail -= n;
            self.cur >> self.avail
        } else {
            let (need, old) = (n - self.avail, self.cur);
            self.cur = m.r8(self.ptr);
            self.ptr = self.ptr.wrapping_add(1);
            self.avail = 8;
            self.read(m, need) | old.wrapping_shl(need as u32)
        };
        r & (0xFFu32 >> (8 - n).clamp(0, 31))
    }
}

/// One tile of the glyph (`0x020336A0`).
struct Tile {
    dst: u32,
    bits: u32,
    xoff: i32,
    yoff: i32,
    gw: i32,
    gh: i32,
    rowbits: i32,
    fbpp: i32,
    cbpp: i32,
    color: u32,
}

fn blit_tile(m: &mut impl Mem, t: &Tile) {
    let colstart = t.xoff.max(0);
    let rowstart = t.yoff.max(0);
    let colend = (t.xoff + t.gw).min(8);
    let rowend = (t.yoff + t.gh).min(8);
    let skiprows = -t.yoff.min(0);
    let skipcols = -t.xoff.min(0);
    let endbits = colend * t.cbpp;
    let startbits = colstart * t.cbpp;
    let mut src = skiprows * t.rowbits + t.fbpp * skipcols;
    let (bytes, cur0) = if t.cbpp == 4 { (4u32, 4) } else { (8, 8) };
    let mut cur = t.dst.wrapping_add((rowstart as u32).wrapping_mul(cur0));
    let end = t.dst.wrapping_add((rowend as u32).wrapping_mul(cur0));
    while cur < end {
        let mut bits = Bits { ptr: t.bits.wrapping_add((src >> 3) as u32), avail: 0, cur: 0 };
        bits.read(m, src & 7);
        if t.cbpp == 4 {
            let mut w = m.r32(cur);
            let mut sh = startbits;
            while sh < endbits {
                let p = bits.read(m, t.fbpp);
                if p != 0 {
                    w = (w & !(0xFu32.wrapping_shl(sh as u32))) | t.color.wrapping_add(p).wrapping_shl(sh as u32);
                }
                sh += 4;
            }
            m.w32(cur, w);
        } else {
            let (mut lo, mut hi) = (m.r32(cur), m.r32(cur.wrapping_add(4)));
            let mut sh = startbits;
            while sh < endbits {
                let p = bits.read(m, t.fbpp);
                if p != 0 {
                    let v = t.color.wrapping_add(p);
                    if sh < 0x20 {
                        lo = (lo & !(0xFFu32.wrapping_shl(sh as u32))) | v.wrapping_shl(sh as u32);
                    } else {
                        hi = (hi & !(0xFFu32.wrapping_shl(sh as u32 - 0x20))) | v.wrapping_shl(sh as u32 - 0x20);
                    }
                }
                sh += 8;
            }
            m.w32(cur, lo);
            m.w32(cur.wrapping_add(4), hi);
        }
        cur = cur.wrapping_add(bytes);
        src += t.rowbits;
    }
}

/// Draws a glyph at (`x`, `y`) of the canvas, clipped to it (`0x020338E0`): `clr` is the colour index of the glyph's
/// pixel value 1, `glyph` the glyph descriptor.
pub fn draw_glyph(m: &mut impl Mem, canvas: u32, font: u32, x: i32, y: i32, clr: u32, glyph: u32) {
    let cbpp = m.r8(canvas + CV_BPP) as i32;
    let tile_bytes = cbpp * 8;
    let info = m.r32(glyph + GL_INFO);
    let gw = m.r8(info + 1) as i32;
    let fres = {
        let f = m.r32(font);
        m.r32(f + FONT_RES)
    };
    let gh = m.r8(fres + RES_CELL_H) as i32;
    let (cw, ch) = (m.r32(canvas + CV_W), m.r32(canvas + CV_H));
    let base = m.r32(canvas + CV_DATA);
    if gw == 0 || x + gw < 0 || y + gh < 0 {
        return;
    }
    let tx0 = if x > 0 { x >> 3 } else { 0 } as u32;
    let ty0 = if y > 0 { y >> 3 } else { 0 } as u32;
    let tx1 = cw.min(((x + gw + 7) as u32) >> 3);
    let ty1 = ch.min(((y + gh + 7) as u32) >> 3);
    let (ncols, nrows) = (tx1 as i32 - tx0 as i32, ty1 as i32 - ty0 as i32);
    if ncols < 0 || nrows <= 0 {
        return;
    }
    let stride = m.r32(canvas + CV_STRIDE);
    let first = stride.wrapping_mul(ty0).wrapping_add(tx0);
    let rowskip = (tile_bytes as u32).wrapping_mul(stride.wrapping_sub(ncols as u32));
    let mut dst = (tile_bytes as u32).wrapping_mul(first).wrapping_add(base);
    let bits = m.r32(glyph + GL_BITS);
    let fbpp = m.r8(fres + RES_BPP) as i32;
    let rowbits = fbpp * m.r8(fres + RES_CELL_W) as i32;
    let xo = if x >= 0 { x & 7 } else { x };
    let yo = if y >= 0 { y & 7 } else { y };
    let (xlim, ylim) = (xo - (ncols << 3), yo - (nrows << 3));
    let mut ry = yo;
    while ry > ylim {
        let mut rx = xo;
        while rx > xlim {
            blit_tile(m, &Tile { dst, bits, xoff: rx, yoff: ry, gw, gh, rowbits, fbpp, cbpp, color: clr.wrapping_sub(1) });
            rx -= 8;
            dst = dst.wrapping_add(tile_bytes as u32);
        }
        dst = dst.wrapping_add(rowskip);
        ry -= 8;
    }
}

/// Guest memory through the machine.
pub struct GuestMem(pub *mut Cpu);

impl Mem for GuestMem {
    fn r8(&mut self, a: u32) -> u32 {
        unsafe { mem_read8(self.0, a) }
    }
    fn r32(&mut self, a: u32) -> u32 {
        unsafe { mem_read32(self.0, a) }
    }
    fn w32(&mut self, a: u32, v: u32) {
        unsafe { mem_write32(self.0, a, v) }
    }
}

/// Guest memory that keeps what is written to it aside: reads see the writes, the guest does not.
pub struct Recorded<M: Mem> {
    pub inner: M,
    pub writes: Vec<(u32, u32)>,
    bytes: HashMap<u32, u8>,
}

impl<M: Mem> Recorded<M> {
    pub fn new(inner: M) -> Self {
        Recorded { inner, writes: Vec::new(), bytes: HashMap::new() }
    }
}

impl<M: Mem> Mem for Recorded<M> {
    fn r8(&mut self, a: u32) -> u32 {
        match self.bytes.get(&a) {
            Some(b) => *b as u32,
            None => self.inner.r8(a),
        }
    }
    fn r32(&mut self, a: u32) -> u32 {
        let a = a & !3;
        if (0..4).any(|i| self.bytes.contains_key(&(a + i))) {
            (0..4).fold(0, |v, i| v | self.r8(a + i) << (8 * i))
        } else {
            self.inner.r32(a)
        }
    }
    fn w32(&mut self, a: u32, v: u32) {
        let a = a & !3;
        for i in 0..4 {
            self.bytes.insert(a + i, (v >> (8 * i)) as u8);
        }
        self.writes.push((a, v));
    }
}

unsafe extern "C" fn hook(c: *mut Cpu) {
    let sp = (*c).r[13];
    let mut m = GuestMem(c);
    let (clr, glyph) = (m.r32(sp), m.r32(sp + 4));
    draw_glyph(&mut m, (*c).r[0], (*c).r[1], (*c).r[2] as i32, (*c).r[3] as i32, clr, glyph);
    // as `pop {..., pc}` of the recompiled code (`arm_iw`)
    let lr = (*c).r[14];
    (*c).pc = if lr & 1 != 0 { lr } else { lr & !3 };
}

/// The key of the guest's glyph blit (ARM9, ARM state).
pub const BLIT_KEY: u32 = 0x0203_38E0;

/// Whether the environment asks for the native blit.
pub fn wanted() -> bool {
    std::env::var("SUNPETAL_NATIVE_DIALOG_DRAW").is_ok_and(|v| v == "1")
}

/// Puts the native blit in place of the guest's.
pub fn install() {
    crate::machine(|m| m.override_fn(9, BLIT_KEY, hook));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat memory for the tests.
    struct Flat(HashMap<u32, u8>);
    impl Mem for Flat {
        fn r8(&mut self, a: u32) -> u32 {
            *self.0.get(&a).unwrap_or(&0) as u32
        }
        fn r32(&mut self, a: u32) -> u32 {
            (0..4).fold(0, |v, i| v | self.r8(a + i) << (8 * i))
        }
        fn w32(&mut self, a: u32, v: u32) {
            for i in 0..4 {
                self.0.insert(a + i, (v >> (8 * i)) as u8);
            }
        }
    }

    fn put32(m: &mut Flat, a: u32, v: u32) {
        m.w32(a, v)
    }

    /// A 4bpp canvas of 4x2 tiles at 0x1000, a font with 8x8 cells of 1 bit per pixel, one glyph "L" (a bar and a foot).
    fn world() -> Flat {
        let mut m = Flat(HashMap::new());
        let (cv, font, fobj, res, info, gl, bitmap) = (0x100, 0x200, 0x204, 0x300, 0x400, 0x500, 0x600);
        for (a, v) in [(cv, 0x1000), (cv + 4, 4), (cv + 8, 2), (cv + 0xC, 4), (cv + 0x10, 4)] {
            put32(&mut m, a, v);
        }
        put32(&mut m, fobj, font);
        put32(&mut m, font + 8, res);
        for (i, b) in [8u8, 8, 0, 0, 0, 0, 1].iter().enumerate() {
            m.0.insert(res + i as u32, *b);
        }
        put32(&mut m, gl, info);
        put32(&mut m, gl + 4, bitmap);
        m.0.insert(info + 1, 5);
        // 8 rows of 8 one-bit pixels: a vertical bar in column 1 and a foot on the last row
        for (i, b) in [0x40u8, 0x40, 0x40, 0x40, 0x40, 0x40, 0x40, 0x7C].iter().enumerate() {
            m.0.insert(bitmap + i as u32, *b);
        }
        m
    }

    fn px(m: &mut Flat, tx: u32, row: u32, col: u32) -> u32 {
        let w = m.r32(0x1000 + tx * 32 + row * 4);
        (w >> (col * 4)) & 0xF
    }

    #[test]
    fn a_glyph_lands_in_its_tile_with_the_colour() {
        let mut m = world();
        draw_glyph(&mut m, 0x100, 0x204, 0, 0, 3, 0x500);
        // colour index 3: pixel value 1 + (3 - 1) = 3
        assert_eq!((px(&mut m, 0, 0, 1), px(&mut m, 0, 0, 0), px(&mut m, 0, 7, 1), px(&mut m, 0, 7, 4)), (3, 0, 3, 3));
        assert_eq!(px(&mut m, 0, 7, 5), 0, "the glyph is five pixels wide");
    }

    #[test]
    fn a_glyph_over_a_tile_edge_goes_into_both_tiles() {
        let mut m = world();
        draw_glyph(&mut m, 0x100, 0x204, 6, 0, 2, 0x500);
        // the bar (glyph column 1) is at canvas column 7, the foot reaches column 10 = tile 1 column 2
        assert_eq!(px(&mut m, 0, 0, 7), 2);
        assert_eq!((px(&mut m, 1, 7, 0), px(&mut m, 1, 7, 2)), (2, 2));
        assert_eq!(px(&mut m, 1, 7, 3), 0);
    }

    #[test]
    fn what_is_outside_the_canvas_is_clipped_and_a_glyph_wholly_outside_draws_nothing() {
        let mut m = world();
        draw_glyph(&mut m, 0x100, 0x204, -20, 0, 2, 0x500);
        draw_glyph(&mut m, 0x100, 0x204, 0, -20, 2, 0x500);
        let before = m.0.len();
        assert!((0..8).all(|r| px(&mut m, 0, r, 1) == 0));
        draw_glyph(&mut m, 0x100, 0x204, 100, 0, 2, 0x500);
        assert_eq!(m.0.len(), before);
        // a glyph over the top edge: only the rows inside
        draw_glyph(&mut m, 0x100, 0x204, 0, -3, 2, 0x500);
        assert_eq!((px(&mut m, 0, 0, 1), px(&mut m, 0, 4, 1), px(&mut m, 0, 5, 1)), (2, 2, 0));
    }

    #[test]
    fn the_recorded_memory_keeps_the_writes_aside() {
        let mut inner = world();
        let mut r = Recorded::new(Flat(inner.0.clone()));
        draw_glyph(&mut r, 0x100, 0x204, 0, 0, 3, 0x500);
        assert!(!r.writes.is_empty());
        assert_eq!(px(&mut inner, 0, 0, 1), 0, "the original is untouched");
        let (a, v) = r.writes[0];
        assert_eq!(r.r32(a), v);
    }

    #[test]
    fn bits_cross_a_byte() {
        let mut m = Flat(HashMap::new());
        m.0.insert(0x10, 0xAB);
        m.0.insert(0x11, 0xCD);
        let mut b = Bits { ptr: 0x10, avail: 0, cur: 0 };
        assert_eq!((b.read(&mut m, 4), b.read(&mut m, 6), b.read(&mut m, 6)), (0xA, 0b101111, 0b001101));
    }
}
