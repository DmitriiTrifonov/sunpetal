//! Texture packs: every texture the game draws has a key (hashes of its data and of the palette it
//! is read through, with its format and size), and a folder of PNG pictures named by key replaces
//! the textures it holds. The same keys name the pictures `dump_textures` writes, so a pack is made
//! by dumping what the game uses, then drawing over it (by hand, by upscaling, by matching with
//! another version's textures).
//!
//! A picture is named `{w}x{h}_{fmt}_{data}_{pal}.png`: the texture's size in texels, its format
//! (1 A3I5, 2 four colours, 3 sixteen, 4 256, 5 compressed 4x4, 6 A5I3, 7 direct), then the two
//! hashes in hex (the palette's is 0 for direct colour). A replacement may have any size (usually
//! a multiple of the original's: it is sampled over the same texture coordinates).
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::render3d::{sample, GxPoly, Image, Mem, TexKey};
use super::{note, Nds};

pub(super) struct TexPack {
    /// Where to write what the game uses (None: nowhere).
    pub dump: Option<PathBuf>,
    /// The replacements.
    pub pack: Arc<HashMap<TexKey, Arc<Image>>>,
    seen: HashSet<TexKey>,
    /// The key of a (TEXIMAGE_PARAM, PLTT_BASE) as of a generation of the texture memory.
    cache: HashMap<(u32, u32), (u64, TexKey)>,
    dumped: usize,
}

impl TexPack {
    pub(super) fn new() -> TexPack {
        TexPack { dump: None, pack: Arc::new(HashMap::new()), seen: HashSet::new(), cache: HashMap::new(), dumped: 0 }
    }

    pub(super) fn active(&self) -> bool {
        self.dump.is_some() || !self.pack.is_empty()
    }
}

fn fnv(bytes: impl IntoIterator<Item = u8>, mut h: u64) -> u64 {
    for b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

const FNV_BASIS: u64 = 0xCBF2_9CE4_8422_2325;

/// A texture's size in texels.
pub(super) fn dims(tex: u32) -> (usize, usize) {
    (8usize << ((tex >> 20) & 7), 8usize << ((tex >> 23) & 7))
}

fn bits_per_texel(fmt: u32) -> usize {
    match fmt {
        2 | 5 => 2,
        3 => 4,
        7 => 16,
        _ => 8,
    }
}

fn tex_byte(m: &Mem, off: usize) -> u8 {
    m.tex.get(off & 0x7FFFF).copied().unwrap_or(0)
}

fn pal_bytes<'a>(m: &'a Mem<'a>, from: usize, n: usize) -> impl Iterator<Item = u8> + 'a {
    (from..from + n).map(move |o| m.pal.get(o).copied().unwrap_or(0))
}

/// The key of a texture as the memory holds it now.
pub(super) fn texture_key(m: &Mem, tex: u32, pal: u32) -> TexKey {
    let fmt = (tex >> 26) & 7;
    let (w, h) = dims(tex);
    let base = ((tex & 0xFFFF) as usize) * 8;
    let c0 = fmt >= 2 && fmt <= 4 && tex & 0x2000_0000 != 0;
    let seed = fnv([fmt as u8, c0 as u8, (w >> 3) as u8, (h >> 3) as u8], FNV_BASIS);
    let n = w * h * bits_per_texel(fmt) / 8;
    let mut data = fnv((0..n).map(|i| tex_byte(m, base + i)), seed);
    let pal16 = pal as usize * 16;
    let palette = match fmt {
        1 => fnv(pal_bytes(m, pal16, 64), FNV_BASIS),
        2 => fnv(pal_bytes(m, pal as usize * 8, 8), FNV_BASIS),
        3 => fnv(pal_bytes(m, pal16, 32), FNV_BASIS),
        4 => fnv(pal_bytes(m, pal16, 512), FNV_BASIS),
        6 => fnv(pal_bytes(m, pal16, 16), FNV_BASIS),
        5 => {
            // the palette parts the tiles' words point to, and the words with the data
            let tiles = (w / 4) * (h / 4);
            let info_at = if base < 0x40000 { 0x20000 + (base >> 1) } else { 0x30000 + ((base - 0x40000) >> 1) };
            let (mut lo, mut hi) = (usize::MAX, 0usize);
            for t in 0..tiles {
                let i = u16::from_le_bytes([tex_byte(m, info_at + t * 2), tex_byte(m, info_at + t * 2 + 1)]);
                data = fnv(i.to_le_bytes(), data);
                let idx = (i & 0x3FFF) as usize;
                lo = lo.min(idx);
                hi = hi.max(idx);
            }
            if lo == usize::MAX { 0 } else { fnv(pal_bytes(m, pal16 + lo * 4, (hi - lo) * 4 + 8), FNV_BASIS) }
        }
        _ => 0,
    };
    TexKey { data, pal: palette, fmt: fmt as u8, w: w as u16, h: h as u16 }
}

/// The picture of a texture, at its own size: what the game would draw, transparent where it
/// would.
pub(super) fn decode(m: &Mem, tex: u32, pal: u32) -> Image {
    let (w, h) = dims(tex);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let (r, g, b, a) = sample(m, tex, pal, x as f32 + 0.5, y as f32 + 0.5);
            rgba.extend_from_slice(&[(r << 2 | r >> 4) as u8, (g << 2 | g >> 4) as u8, (b << 2 | b >> 4) as u8, (a * 255 / 31) as u8]);
        }
    }
    Image { w, h, rgba }
}

pub(super) fn file_name(k: &TexKey) -> String {
    format!("{}x{}_{}_{:016x}_{:016x}.png", k.w, k.h, k.fmt, k.data, k.pal)
}

pub(super) fn parse_name(name: &str) -> Option<TexKey> {
    let stem = name.strip_suffix(".png")?;
    let mut it = stem.split('_');
    let (size, fmt, data, pal) = (it.next()?, it.next()?, it.next()?, it.next()?);
    if it.next().is_some() {
        return None;
    }
    let (w, h) = size.split_once('x')?;
    Some(TexKey {
        data: u64::from_str_radix(data, 16).ok()?,
        pal: u64::from_str_radix(pal, 16).ok()?,
        fmt: fmt.parse().ok()?,
        w: w.parse().ok()?,
        h: h.parse().ok()?,
    })
}

fn write_png(path: &Path, img: &Image) -> Result<(), String> {
    let f = fs::File::create(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), img.w as u32, img.h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(&img.rgba).map_err(|e| e.to_string())
}

fn read_png(path: &Path) -> Result<Image, String> {
    let f = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(f));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (w, h) = (info.width as usize, info.height as usize);
    let px = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => px.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("indexed picture not expanded".into()),
    };
    if w == 0 || h == 0 || rgba.len() != w * h * 4 {
        return Err("empty or odd picture".into());
    }
    Ok(Image { w, h, rgba })
}

impl Nds {
    /// Reads a folder of replacements (pictures named as `file_name` makes them).
    pub(super) fn load_pack(&mut self, dir: &Path) -> Result<(), String> {
        let rd = fs::read_dir(dir).map_err(|e| format!("texture pack {}: {}", dir.display(), e))?;
        let mut pack = HashMap::new();
        let mut skipped = 0;
        for e in rd.flatten() {
            let p = e.path();
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else { continue };
            let Some(key) = parse_name(name) else {
                if name.ends_with(".png") {
                    skipped += 1;
                }
                continue;
            };
            match read_png(&p) {
                Ok(img) => {
                    pack.insert(key, Arc::new(img));
                }
                Err(e) => {
                    skipped += 1;
                    note(&format!("texture pack: {}: {}", name, e));
                }
            }
        }
        note(&format!("texture pack {}: {} textures{}", dir.display(), pack.len(), if skipped != 0 { format!(", {} pictures skipped", skipped) } else { String::new() }));
        self.texpack.pack = Arc::new(pack);
        Ok(())
    }

    /// The keys of the textures a frame's polygons use (cached while the texture memory does not
    /// change), and the dump of the ones not seen before.
    pub(super) fn texture_keys(&mut self, polys: &[GxPoly], mem: &Mem, texture_on: bool) -> HashMap<(u32, u32), TexKey> {
        let mut keys = HashMap::new();
        if !texture_on || !self.texpack.active() {
            return keys;
        }
        let gen = self.tex_generation();
        for p in polys {
            if (p.tex >> 26) & 7 == 0 {
                continue;
            }
            let id = (p.tex, p.pal);
            if keys.contains_key(&id) {
                continue;
            }
            let key = match self.texpack.cache.get(&id) {
                Some((g, k)) if *g == gen => *k,
                _ => {
                    let k = texture_key(mem, p.tex, p.pal);
                    self.texpack.cache.insert(id, (gen, k));
                    k
                }
            };
            keys.insert(id, key);
            if let Some(dir) = self.texpack.dump.clone() {
                if self.texpack.seen.insert(key) {
                    let img = decode(mem, p.tex, p.pal);
                    let name = file_name(&key);
                    let frame = self.vblanks;
                    self.texpack.dumped += 1;
                    let n = self.texpack.dumped;
                    // written off to the side: the machine does not wait for the disk
                    std::thread::spawn(move || {
                        let _ = fs::create_dir_all(&dir);
                        let path = dir.join(&name);
                        if !path.exists() {
                            if let Err(e) = write_png(&path, &img) {
                                eprintln!("texture dump: {}", e);
                                return;
                            }
                        }
                        // one write: lines from several threads do not mix
                        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(dir.join("index.csv")) {
                            let line = format!("{},{},{},{},{},{}\n", name, key.fmt, key.w, key.h, frame, n);
                            let _ = f.write_all(line.as_bytes());
                        }
                    });
                }
            }
        }
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        let k = TexKey { data: 0x0123_4567_89AB_CDEF, pal: 0xFEDC_BA98_7654_3210, fmt: 5, w: 64, h: 32 };
        let n = file_name(&k);
        assert_eq!(n, "64x32_5_0123456789abcdef_fedcba9876543210.png");
        assert_eq!(parse_name(&n), Some(k));
        assert_eq!(parse_name("readme.txt"), None);
        assert_eq!(parse_name("64x32_5_zz_00.png"), None);
        assert_eq!(parse_name("64x32_5_00_00_extra.png"), None);
    }

    fn texture(fmt: u32, size: u32, base: u32) -> u32 {
        fmt << 26 | size << 23 | size << 20 | base
    }

    #[test]
    fn a_key_follows_the_data_and_the_palette() {
        let (mut tex, mut pal) = (vec![0u8; 0x80000], vec![0u8; 0x18000]);
        let t = texture(4, 0, 0); // 8x8, 256 colours
        fn m<'a>(tex: &'a [u8], pal: &'a [u8]) -> Mem<'a> {
            Mem { tex, pal }
        }
        let k0 = texture_key(&m(&tex, &pal), t, 0);
        assert_eq!((k0.w, k0.h, k0.fmt), (8, 8, 4));
        tex[10] = 7; // a texel inside it
        let k1 = texture_key(&m(&tex, &pal), t, 0);
        assert_ne!(k0.data, k1.data);
        assert_eq!(k0.pal, k1.pal);
        pal[4] = 1; // a palette entry
        let k2 = texture_key(&m(&tex, &pal), t, 0);
        assert_eq!(k1.data, k2.data);
        assert_ne!(k1.pal, k2.pal);
        tex[8 * 8] = 9; // past its end: not part of it
        assert_eq!(texture_key(&m(&tex, &pal), t, 0), k2);
    }

    #[test]
    fn decode_gives_the_games_texels() {
        let mut tex = vec![0u8; 0x80000];
        tex[0] = 0x1F; // direct colour, texel 0: red (5 bits) ...
        tex[1] = 0x80; // ... and the alpha bit
        let pal = vec![0u8; 0x18000];
        let img = decode(&Mem { tex: &tex, pal: &pal }, texture(7, 0, 0), 0);
        assert_eq!((img.w, img.h), (8, 8));
        assert_eq!(&img.rgba[..4], &[255, 0, 0, 255]);
        assert_eq!(img.rgba[7], 0, "the next texel is transparent");
    }
}
