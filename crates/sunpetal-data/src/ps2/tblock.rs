//! The texture block of a scene or character record and the choice of a material's picture (У1.5).
//!
//! A `Scene3D` record keeps the offset of its T-block in the `u32` at 0x1C, a `Chara` record in the one at 4.
//! The T-block is a count and `(offset, size)` entries; each entry is a run of GS uploads (a picture, then
//! its palette). A material names its picture by TEX0: `TBP0` (where the picture sits in the GS memory),
//! `CBP` (its palette), the size `1<<TW` × `1<<TH` and the buffer width. `pick` finds the upload for it;
//! a T8 picture that the game builds from several uploads into one page is read out of an assembled model
//! of the GS memory (`assemble`, `material_pixels`).
use super::gs::{self, Upload, PSM_CT32, PSM_T4, PSM_T8};
use std::collections::HashMap;

/// Which record the T-block belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Scene3D,
    Chara,
}

impl Role {
    fn field(self) -> usize {
        match self {
            Role::Scene3D => 0x1C,
            Role::Chara => 4,
        }
    }
}

/// The parts of a TEX0 register a material needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tex0 {
    pub tbp0: u32,
    pub tbw: u32,
    pub psm: u8,
    pub tw: u32,
    pub th: u32,
    pub cbp: u32,
}

impl Tex0 {
    pub fn from_bits(v: u64) -> Tex0 {
        Tex0 {
            tbp0: v as u32 & 0x3FFF,
            tbw: (v >> 14) as u32 & 63,
            psm: (v >> 20) as u8 & 63,
            tw: (v >> 26) as u32 & 15,
            th: (v >> 30) as u32 & 15,
            cbp: (v >> 37) as u32 & 0x3FFF,
        }
    }
    pub fn size(&self) -> (u32, u32) {
        (1 << self.tw, 1 << self.th)
    }
}

fn u32at(d: &[u8], p: usize) -> Option<usize> {
    Some(u32::from_le_bytes(d.get(p..p + 4)?.try_into().ok()?) as usize)
}

/// All uploads of the record's T-block, entry after entry (offsets index `rec`). A record without a T-block gives none.
pub fn uploads(rec: &[u8], role: Role) -> Result<Vec<Upload>, String> {
    let t = u32at(rec, role.field()).ok_or("record shorter than its header")?;
    if t == 0 {
        return Ok(Vec::new());
    }
    uploads_at(rec, t)
}

/// The uploads of the T-block at `t` in `rec`: a count and `(offset, size)` entries counted from `t`.
fn uploads_at(rec: &[u8], t: usize) -> Result<Vec<Upload>, String> {
    let n = u32at(rec, t).ok_or("T-block past the record")?;
    let mut out = Vec::new();
    for e in 0..n {
        let (eo, es) = (u32at(rec, t + 4 + 8 * e).ok_or("T-block entry past the record")?, u32at(rec, t + 8 + 8 * e).ok_or("T-block entry past the record")?);
        let (from, to) = (t + eo, t + eo + es);
        if to > rec.len() {
            return Err(format!("T-block entry {e} past the record"));
        }
        out.extend(gs::uploads(rec, from..to));
    }
    Ok(out)
}

/// An item model or character shadow of a `Chara` record with its own T-block.
#[derive(Debug)]
pub struct ItemTex {
    /// Offset of the model's GIG in the record.
    pub gig: usize,
    pub uploads: Vec<Upload>,
}

/// The item table of a `Chara` record (`WCharaItemDataHeader`, the `u32` at 0x14): a count and
/// `(gig offset, T-block offset)` pairs counted from the table. The record's main T-block (at 4) does not
/// belong to these models. Empty pairs (both 0) are skipped. A page is built from one model's own T-block only:
/// the game reuses GS page addresses between models, so blocks from other records are other pictures.
pub fn item_uploads(rec: &[u8]) -> Result<Vec<ItemTex>, String> {
    let t = u32at(rec, 0x14).ok_or("record shorter than its header")?;
    if t == 0 {
        return Ok(Vec::new());
    }
    let n = u32at(rec, t).ok_or("item table past the record")?;
    let mut out = Vec::new();
    for k in 0..n {
        let (g, x) = (u32at(rec, t + 4 + 8 * k).ok_or("item entry past the record")?, u32at(rec, t + 8 + 8 * k).ok_or("item entry past the record")?);
        if g == 0 && x == 0 {
            continue;
        }
        let uploads = if x == 0 { Vec::new() } else { uploads_at(rec, t + x)? };
        out.push(ItemTex { gig: t + g, uploads });
    }
    Ok(out)
}

const BLK32: [[u32; 8]; 4] = [[0, 1, 4, 5, 16, 17, 20, 21], [2, 3, 6, 7, 18, 19, 22, 23], [8, 9, 12, 13, 24, 25, 28, 29], [10, 11, 14, 15, 26, 27, 30, 31]];

/// The GS block (in `CBP` units) of the upper left corner of a CT32 upload: how a palette is addressed.
pub fn clut_block(u: &Upload) -> u32 {
    let (bx, by) = (u.dsax / 8, u.dsay / 8);
    let page = (u.dsay / 32) * u.dbw.max(1) + u.dsax / 64;
    u.dbp + 32 * page + BLK32[(by % 4) as usize][(bx % 8) as usize]
}

/// The picture and palette uploads for a material. Priority for the picture: the exact `TBP0` and size, then
/// the `TBP0`, then the range of blocks it lies in; the last upload of a kind wins (a later one overwrites).
pub fn pick<'a>(ups: &'a [Upload], t: &Tex0) -> Option<(&'a Upload, Option<&'a Upload>)> {
    let (w, h) = t.size();
    let imgs: Vec<&Upload> = ups.iter().filter(|u| u.psm != PSM_CT32 || u.w * u.h > 16).collect();
    let by = |f: &dyn Fn(&&Upload) -> bool| imgs.iter().copied().filter(f).collect::<Vec<_>>();
    let mut cand = by(&|u| u.dbp == t.tbp0 && u.w == w && u.h == h);
    if cand.is_empty() {
        cand = by(&|u| u.dbp == t.tbp0);
    }
    if cand.is_empty() {
        cand = by(&|u| u.dbp <= t.tbp0 && t.tbp0 < u.dbp + (u.w * u.h / 64).max(1));
    }
    let im = *cand.last()?;
    let clut = match im.psm {
        PSM_T8 => Some(ups.iter().rfind(|u| u.psm == PSM_CT32 && u.w == 16 && u.h == 16 && clut_block(u) == t.cbp)?),
        PSM_T4 => Some(ups.iter().rfind(|u| u.psm == PSM_CT32 && u.w * u.h == 16 && (clut_block(u) == t.cbp || u.dbp == t.cbp))?),
        _ => None,
    };
    Some((im, clut))
}

/// The GS memory as 16×16-pixel blocks of palette indices, built from the record's T8 uploads.
pub type Memory = HashMap<u32, [u8; 256]>;

fn page_block(bp: u32, bw: u32, x: u32, y: u32) -> u32 {
    let page = (y / 64) * (bw / 2).max(1) + x / 128;
    bp + 32 * page + BLK32[((y % 64) / 16) as usize][((x % 128) / 16) as usize]
}

/// Puts every whole-row T8 upload into a model of the memory.
pub fn assemble(rec: &[u8], ups: &[Upload]) -> Memory {
    let mut mem = Memory::new();
    for u in ups.iter().filter(|u| u.psm == PSM_T8 && (u.w * u.h) as usize == u.data.len()) {
        let Some(data) = rec.get(u.data.clone()) else { continue };
        for y in 0..u.h {
            for x in 0..u.w {
                let (sx, sy) = (u.dsax + x, u.dsay + y);
                let b = mem.entry(page_block(u.dbp, u.dbw, sx, sy)).or_insert([0; 256]);
                b[((sy % 16) * 16 + sx % 16) as usize] = data[(y * u.w + x) as usize];
            }
        }
    }
    mem
}

/// The indices of a T8 material read out of the assembled memory (the TEX0 rectangle), the count of pixels
/// whose block was never uploaded, and the palette upload; `None` when nothing of it is there or the palette is.
pub fn material_pixels<'a>(mem: &Memory, ups: &'a [Upload], t: &Tex0) -> Option<(Vec<u8>, usize, &'a Upload)> {
    let (w, h) = t.size();
    let hbw = (t.tbw / 2).max(1);
    let (mut px, mut miss) = (vec![0u8; (w * h) as usize], 0);
    for y in 0..h {
        let row = &BLK32[((y % 64) / 16) as usize];
        let page0 = t.tbp0 + 32 * (y / 64) * hbw;
        for x in 0..w {
            match mem.get(&(page0 + 32 * (x / 128) + row[((x % 128) / 16) as usize])) {
                Some(b) => px[(y * w + x) as usize] = b[((y % 16) * 16 + x % 16) as usize],
                None => miss += 1,
            }
        }
    }
    if miss == (w * h) as usize {
        return None;
    }
    let clut = ups.iter().rfind(|u| u.psm == PSM_CT32 && u.w == 16 && u.h == 16 && clut_block(u) == t.cbp)?;
    Some((px, miss, clut))
}

/// The key of a T8 material's picture in `SPM2` and the texture pack (`spm2.rs`): the first 8 bytes (big endian)
/// of the SHA-1 of the TEX0 rectangle's indices and of the palette upload's bytes. The export's picture id
/// (`<sha1 of the indices>[:10]_<sha1 of the palette>[:8]`, `sunpetal-ee`) is the prefix of the same hashes.
pub fn material_key(rec: &[u8], mem: &Memory, ups: &[Upload], t: &Tex0) -> Option<(u64, u64)> {
    let (px, _, clut) = material_pixels(mem, ups, t)?;
    let head = |b: &[u8]| u64::from_str_radix(&super::sha1::sha1_hex(b)[..16], 16).ok();
    Some((head(&px)?, head(rec.get(clut.data.clone())?)?))
}

/// RGBA of a material: from its uploads (`pick`), or for T8 from the assembled memory when `from_memory`.
pub fn material_rgba(rec: &[u8], ups: &[Upload], mem: &Memory, t: &Tex0) -> Result<Option<Vec<u8>>, String> {
    if t.psm == PSM_T8 {
        let Some((px, _, clut)) = material_pixels(mem, ups, t) else { return Ok(None) };
        let cols = gs::clut_colors(rec, clut)?;
        return Ok(Some(px.iter().flat_map(|&i| cols[i as usize]).collect()));
    }
    match pick(ups, t) {
        Some((im, clut)) => gs::to_rgba(rec, im, clut).map(Some),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn up(dbp: u32, dbw: u32, psm: u8, w: u32, h: u32) -> Upload {
        Upload { at: 0, dbp, dbw, psm, w, h, dsax: 0, dsay: 0, data: 0..(w * h) as usize }
    }

    #[test]
    fn tex0_fields_are_cut_from_the_register() {
        let v = 0x20052c05dd352940u64;
        let t = Tex0::from_bits(v);
        assert_eq!((t.tbp0, t.psm, t.tw, t.th, t.cbp, t.tbw), (10560, 19, 7, 7, 10592, 20));
        assert_eq!(t.size(), (128, 128));
    }

    #[test]
    fn pick_prefers_the_exact_size_and_the_last_upload() {
        let ups = vec![up(100, 2, PSM_T8, 64, 64), up(100, 2, PSM_T8, 128, 128), up(100, 2, PSM_T8, 128, 128), up(200, 1, PSM_CT32, 16, 16)];
        let t = Tex0 { tbp0: 100, tbw: 2, psm: 19, tw: 7, th: 7, cbp: 200 };
        let (im, clut) = pick(&ups, &t).unwrap();
        assert!(std::ptr::eq(im, &ups[2]));
        assert!(std::ptr::eq(clut.unwrap(), &ups[3]));
        // without the palette at CBP the material has no picture
        assert!(pick(&ups, &Tex0 { cbp: 300, ..t }).is_none());
        // without the exact size the TBP0 decides
        assert!(std::ptr::eq(pick(&ups, &Tex0 { tw: 5, th: 5, ..t }).unwrap().0, &ups[2]));
    }

    #[test]
    fn a_palette_is_addressed_by_its_block() {
        let mut u = up(1000, 1, PSM_CT32, 16, 16);
        assert_eq!(clut_block(&u), 1000);
        u.dsax = 8;
        assert_eq!(clut_block(&u), 1001);
        u.dsay = 8;
        assert_eq!(clut_block(&u), 1003);
    }

    #[test]
    fn t8_pixels_come_back_from_the_assembled_page() {
        // a 16x16 picture at TBP0 100 and its palette at 200: indices 0..=255 in raster order
        let mut rec: Vec<u8> = (0..=255u8).collect();
        let pal: Vec<u8> = (0..256usize).flat_map(|i| [i as u8, 0, 0, 64]).collect();
        let pal_at = rec.len();
        rec.extend(&pal);
        let mut img = up(100, 2, PSM_T8, 16, 16);
        img.data = 0..256;
        let mut clut = up(200, 1, PSM_CT32, 16, 16);
        clut.data = pal_at..pal_at + 1024;
        let ups = vec![img, clut];
        let mem = assemble(&rec, &ups);
        let t = Tex0 { tbp0: 100, tbw: 2, psm: 19, tw: 4, th: 4, cbp: 200 };
        let (px, miss, _) = material_pixels(&mem, &ups, &t).unwrap();
        assert_eq!((miss, px), (0, (0..=255u8).collect::<Vec<_>>()));
        // a rectangle nothing was uploaded to has no picture
        assert!(material_pixels(&mem, &ups, &Tex0 { tbp0: 5000, ..t }).is_none());
        let rgba = material_rgba(&rec, &ups, &mem, &t).unwrap().unwrap();
        assert_eq!(rgba.len(), 1024);
    }

    #[test]
    fn the_t_block_is_read_from_the_role_s_field() {
        let mut rec = vec![0u8; 0x40];
        assert!(uploads(&rec, Role::Scene3D).unwrap().is_empty());
        rec[4..8].copy_from_slice(&0x1000u32.to_le_bytes());
        assert!(uploads(&rec, Role::Chara).is_err());
        assert!(uploads(&rec[..2], Role::Chara).is_err());
    }
}
