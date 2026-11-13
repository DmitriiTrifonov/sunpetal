//! GS image uploads and their pixels (У1.4). A record of the disc carries finished uploads to the GS:
//! a GIF tag of four A+D registers (BITBLTBUF, TRXPOS, TRXREG, TRXDIR), a GIF tag of the IMAGE kind and the
//! data. The data is the raster as transferred (no swizzle to undo here); a palette is an upload too.
//! Formats: CT32 (alpha 0..128 doubled), CT16 and CT16S, T8, T4; a 256-colour palette is stored in the CSM1
//! order, swapping bits 3 and 4 of the index.
use std::ops::Range;

pub const PSM_CT32: u8 = 0x00;
pub const PSM_CT16: u8 = 0x02;
pub const PSM_CT16S: u8 = 0x0A;
pub const PSM_T8: u8 = 0x13;
pub const PSM_T4: u8 = 0x14;

/// The GIF tag of four A+D registers that opens every upload.
const SIG: [u8; 16] = [0x04, 0, 0, 0, 0, 0, 0, 0x10, 0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];

/// One upload: where it goes in the GS memory and where its data lies in the source bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upload {
    /// Offset of the A+D tag in the source.
    pub at: usize,
    pub dbp: u32,
    pub dbw: u32,
    pub psm: u8,
    pub w: u32,
    pub h: u32,
    pub dsax: u32,
    pub dsay: u32,
    /// The data's place in the source.
    pub data: Range<usize>,
}

impl Upload {
    /// A CT32 or CT16 upload of 16 or 256 pixels: a palette, not a picture.
    pub fn is_clut(&self) -> bool {
        matches!(self.psm, PSM_CT32 | PSM_CT16 | PSM_CT16S) && (self.w * self.h == 16 || self.w * self.h == 256)
    }
}

fn u64at(d: &[u8], p: usize) -> Option<u64> {
    Some(u64::from_le_bytes(d.get(p..p + 8)?.try_into().ok()?))
}

/// The upload whose A+D tag lies at `p`, and the offset after its data.
fn packet(d: &[u8], p: usize) -> Option<(Upload, usize)> {
    let (lo, hi) = (u64at(d, p)?, u64at(d, p + 8)?);
    let (nloop, flg, nreg) = ((lo & 0x7FFF) as usize, (lo >> 58) & 3, (lo >> 60) & 15);
    if flg != 0 || hi & 15 != 0xE || nreg != 1 {
        return None;
    }
    let (mut bb, mut trx, mut pos) = (None, None, 0u64);
    let mut q = p + 16;
    for _ in 0..nloop {
        let (val, addr) = (u64at(d, q)?, u64at(d, q + 8)?);
        match addr & 0xFF {
            0x50 => bb = Some(val),
            0x51 => pos = val,
            0x52 => trx = Some(val),
            _ => {}
        }
        q += 16;
    }
    let (bb, trx) = (bb?, trx?);
    let lo2 = u64at(d, q)?;
    if (lo2 >> 58) & 3 != 2 {
        return None;
    }
    let n = (lo2 & 0x7FFF) as usize * 16;
    q += 16;
    if q + n > d.len() {
        return None;
    }
    let up = Upload {
        at: p,
        dbp: (bb >> 32) as u32 & 0x3FFF,
        dbw: (bb >> 48) as u32 & 0x3F,
        psm: (bb >> 56) as u8 & 0x3F,
        w: trx as u32,
        h: (trx >> 32) as u32,
        dsax: (pos >> 32) as u32 & 0x7FF,
        dsay: (pos >> 48) as u32 & 0x7FF,
        data: q..q + n,
    };
    Some((up, q + n))
}

/// Every upload in `d[range]`, found by the signature of its tag.
pub fn uploads(d: &[u8], range: Range<usize>) -> Vec<Upload> {
    let stop = range.end.min(d.len());
    let (mut out, mut p) = (Vec::new(), range.start);
    while p + 16 <= stop {
        match d[p..stop].windows(16).position(|w| w == SIG) {
            None => break,
            Some(i) => p += i,
        }
        match packet(&d[..stop], p) {
            Some((u, end)) => {
                out.push(u);
                p = end;
            }
            None => p += 16,
        }
    }
    out
}

fn px16(v: u16) -> [u8; 4] {
    let (r, g, b) = ((v & 31) as u8, (v >> 5 & 31) as u8, (v >> 10 & 31) as u8);
    [r << 3 | r >> 2, g << 3 | g >> 2, b << 3 | b >> 2, if v & 0x8000 != 0 { 255 } else { 0 }]
}

/// The colours of a palette upload (CSM1 order undone for 256 colours).
pub fn clut_colors(d: &[u8], u: &Upload) -> Result<Vec<[u8; 4]>, String> {
    let n = (u.w * u.h) as usize;
    let data = d.get(u.data.clone()).ok_or("palette data out of range")?;
    let mut cols: Vec<[u8; 4]> = match u.psm {
        PSM_CT32 if data.len() >= 4 * n => (0..n).map(|i| [data[4 * i], data[4 * i + 1], data[4 * i + 2], data[4 * i + 3].saturating_mul(2)]).collect(),
        PSM_CT16 | PSM_CT16S if data.len() >= 2 * n => (0..n).map(|i| px16(u16::from_le_bytes([data[2 * i], data[2 * i + 1]]))).collect(),
        _ => return Err(format!("palette format {:#x} with {} bytes for {n} colours", u.psm, data.len())),
    };
    if n == 256 {
        cols = (0..256).map(|i| cols[(i & !0x18) | ((i & 8) << 1) | ((i & 16) >> 1)]).collect();
    }
    Ok(cols)
}

/// RGBA bytes of an image upload; `clut` is the palette for T8 and T4.
pub fn to_rgba(d: &[u8], u: &Upload, clut: Option<&Upload>) -> Result<Vec<u8>, String> {
    let n = (u.w * u.h) as usize;
    let data = d.get(u.data.clone()).ok_or("image data out of range")?;
    let need = match u.psm {
        PSM_CT32 => 4 * n,
        PSM_CT16 | PSM_CT16S => 2 * n,
        PSM_T8 => n,
        PSM_T4 => n.div_ceil(2),
        p => return Err(format!("image format {p:#x}")),
    };
    if data.len() < need {
        return Err(format!("image data {} bytes, {need} needed", data.len()));
    }
    let mut out = Vec::with_capacity(4 * n);
    match u.psm {
        PSM_CT32 => (0..n).for_each(|i| out.extend([data[4 * i], data[4 * i + 1], data[4 * i + 2], data[4 * i + 3].saturating_mul(2)])),
        PSM_CT16 | PSM_CT16S => (0..n).for_each(|i| out.extend(px16(u16::from_le_bytes([data[2 * i], data[2 * i + 1]])))),
        _ => {
            let cols = clut_colors(d, clut.ok_or("an indexed image needs a palette")?)?;
            let at = |i: usize| cols.get(i).copied().ok_or_else(|| format!("index {i} past a palette of {}", cols.len()));
            for i in 0..n {
                let ix = if u.psm == PSM_T8 { data[i] as usize } else { (data[i >> 1] >> (4 * (i & 1)) & 15) as usize };
                out.extend(at(ix)?);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upload(dbp: u32, psm: u8, w: u32, h: u32, data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend([0x04, 0, 0, 0, 0, 0, 0, 0x10, 0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        let bb = (dbp as u64) << 32 | 1 << 48 | (psm as u64) << 56;
        for (val, reg) in [(bb, 0x50u64), (0, 0x51), (w as u64 | (h as u64) << 32, 0x52), (0, 0x53)] {
            v.extend(val.to_le_bytes());
            v.extend(reg.to_le_bytes());
        }
        let qwc = data.len().div_ceil(16) as u64;
        v.extend((qwc | 2 << 58).to_le_bytes());
        v.extend(0u64.to_le_bytes());
        v.extend(data);
        v.resize(v.len().div_ceil(16) * 16, 0);
        v
    }

    #[test]
    fn uploads_are_found_between_other_bytes() {
        let mut d = vec![7u8; 5];
        d.extend(upload(0x100, PSM_CT32, 2, 1, &[1, 2, 3, 4, 5, 6, 7, 8]));
        d.extend([9u8; 3]);
        d.extend(upload(0x200, PSM_T8, 4, 1, &[0, 1, 0, 1]));
        let ups = uploads(&d, 0..d.len());
        assert_eq!(ups.iter().map(|u| (u.dbp, u.psm, u.w, u.h, u.data.len())).collect::<Vec<_>>(), [(0x100, 0, 2, 1, 16), (0x200, 0x13, 4, 1, 16)]);
        assert_eq!(&d[ups[0].data.clone()][..8], &[1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn ct32_alpha_is_doubled_and_ct16_widened() {
        let d = upload(0, PSM_CT32, 1, 1, &[10, 20, 30, 64]);
        let u = &uploads(&d, 0..d.len())[0];
        assert_eq!(to_rgba(&d, u, None).unwrap(), [10, 20, 30, 128]);
        let d = upload(0, PSM_CT16, 2, 1, &[0xFF, 0xFF, 0x1F, 0x00]);
        let u = &uploads(&d, 0..d.len())[0];
        assert_eq!(to_rgba(&d, u, None).unwrap(), [255, 255, 255, 255, 255, 0, 0, 0]);
    }

    #[test]
    fn t8_and_t4_take_their_colours_from_the_palette() {
        // a 16-colour palette: colour i = (i, 0, 0, 128)
        let pal: Vec<u8> = (0..16u8).flat_map(|i| [i, 0, 0, 64]).collect();
        let mut d = upload(0, PSM_CT32, 8, 2, &pal);
        let t4_at = d.len();
        d.extend(upload(0, PSM_T4, 4, 1, &[0x21, 0x43]));
        let ups = uploads(&d, 0..d.len());
        assert!(ups[0].is_clut() && !ups[1].is_clut());
        assert_eq!(ups[1].at, t4_at);
        let rgba = to_rgba(&d, &ups[1], Some(&ups[0])).unwrap();
        assert_eq!(rgba.chunks(4).map(|p| p[0]).collect::<Vec<_>>(), [1, 2, 3, 4]);
        assert!(to_rgba(&d, &ups[1], None).is_err());
    }

    #[test]
    fn a_256_palette_has_bits_3_and_4_swapped() {
        let pal: Vec<u8> = (0..256usize).flat_map(|i| [i as u8, 0, 0, 0]).collect();
        let d = upload(0, PSM_CT32, 16, 16, &pal);
        let c = clut_colors(&d, &uploads(&d, 0..d.len())[0]).unwrap();
        assert_eq!((c[0][0], c[8][0], c[16][0], c[24][0], c[255][0]), (0, 16, 8, 24, 255));
    }

    #[test]
    fn short_data_is_an_error_not_a_panic() {
        let mut d = upload(0, PSM_T8, 4, 4, &[0; 16]);
        let u = uploads(&d, 0..d.len()).remove(0);
        d.truncate(u.data.start + 4);
        assert!(to_rgba(&d, &u, None).is_err());
    }
}
