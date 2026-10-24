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
use std::io::{Read, Seek, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use super::render3d::{sample, GxPoly, Image, Mem, TexKey};
use super::{note, Nds};

pub(super) struct TexPack {
    /// Where to write what the game uses (None: nowhere).
    pub dump: Option<PathBuf>,
    /// The replacements.
    pub pack: Arc<Pack>,
    seen: HashSet<TexKey>,
    /// The key of a (TEXIMAGE_PARAM, PLTT_BASE) as of a generation of the texture memory.
    cache: HashMap<(u32, u32), (u64, TexKey)>,
    dumped: usize,
}

impl TexPack {
    pub(super) fn new() -> TexPack {
        TexPack { dump: None, pack: Arc::new(Pack::empty()), seen: HashSet::new(), cache: HashMap::new(), dumped: 0 }
    }

    pub(super) fn active(&self) -> bool {
        self.dump.is_some() || !self.pack.is_empty()
    }
}

/// The replacement pictures, from a folder of PNGs or a data file (`tools/ps2/spdata.py`). Each is
/// read and decoded the first time the game draws its texture, so a pack of thousands costs only
/// what is used.
pub(super) struct Pack {
    entries: HashMap<TexKey, Entry>,
    /// The data file the `Slice` entries are read from.
    file: Option<Mutex<fs::File>>,
}

enum Source {
    Path(PathBuf),
    Slice { off: u64, len: u32 },
}

struct Entry {
    source: Source,
    image: OnceLock<Option<Arc<Image>>>,
}

impl Pack {
    pub(super) fn empty() -> Pack {
        Pack { entries: HashMap::new(), file: None }
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The picture that replaces a texture, if the pack has one.
    pub(super) fn get(&self, key: &TexKey) -> Option<Arc<Image>> {
        let e = self.entries.get(key)?;
        e.image
            .get_or_init(|| {
                let bytes = match &e.source {
                    Source::Path(p) => fs::read(p).map_err(|x| x.to_string()),
                    Source::Slice { off, len } => self.read_slice(*off, *len),
                };
                match bytes.and_then(|b| decode_png(&b)) {
                    Ok(img) => Some(Arc::new(img)),
                    Err(x) => {
                        note(&format!("texture pack: {}: {}", file_name(key), x));
                        None
                    }
                }
            })
            .clone()
    }

    fn read_slice(&self, off: u64, len: u32) -> Result<Vec<u8>, String> {
        let mut f = self.file.as_ref().ok_or("no data file")?.lock().map_err(|_| "data file lock")?;
        f.seek(SeekFrom::Start(off)).map_err(|e| e.to_string())?;
        let mut buf = vec![0; len as usize];
        f.read_exact(&mut buf).map_err(|e| e.to_string())?;
        Ok(buf)
    }

    /// A folder of pictures named by `file_name`.
    pub(super) fn from_dir(dir: &Path) -> Result<Pack, String> {
        let rd = fs::read_dir(dir).map_err(|e| format!("texture pack {}: {}", dir.display(), e))?;
        let mut entries = HashMap::new();
        for e in rd.flatten() {
            let p = e.path();
            if let Some(key) = p.file_name().and_then(|n| n.to_str()).and_then(parse_name) {
                entries.insert(key, Entry { source: Source::Path(p), image: OnceLock::new() });
            }
        }
        Ok(Pack { entries, file: None })
    }

    /// A data file: the index is read now, the pictures later.
    pub(super) fn from_file(path: &Path) -> Result<Pack, String> {
        let bad = |why: &str| format!("data file {}: {}", path.display(), why);
        let mut f = fs::File::open(path).map_err(|e| bad(&e.to_string()))?;
        let mut head = [0u8; 12];
        f.read_exact(&mut head).map_err(|_| bad("too short"))?;
        if &head[..8] != b"SPDATA\0\x01" {
            return Err(bad("not a Sunpetal data file, or a newer version of it"));
        }
        let sections = u32::from_le_bytes(head[8..12].try_into().unwrap());
        let mut texs = None;
        for _ in 0..sections.min(64) {
            let mut s = [0u8; 20];
            f.read_exact(&mut s).map_err(|_| bad("section table cut short"))?;
            if &s[..4] == b"TEXS" {
                texs = Some(u64::from_le_bytes(s[4..12].try_into().unwrap()));
            }
        }
        let mut entries = HashMap::new();
        if let Some(at) = texs {
            f.seek(SeekFrom::Start(at)).map_err(|e| bad(&e.to_string()))?;
            let mut r = std::io::BufReader::new(&mut f);
            let mut n = [0u8; 4];
            r.read_exact(&mut n).map_err(|_| bad("texture index cut short"))?;
            for _ in 0..u32::from_le_bytes(n) {
                let mut e = [0u8; 33];
                r.read_exact(&mut e).map_err(|_| bad("texture index cut short"))?;
                let key = TexKey {
                    data: u64::from_le_bytes(e[0..8].try_into().unwrap()),
                    pal: u64::from_le_bytes(e[8..16].try_into().unwrap()),
                    fmt: e[16],
                    w: u16::from_le_bytes([e[17], e[18]]),
                    h: u16::from_le_bytes([e[19], e[20]]),
                };
                let source = Source::Slice { off: u64::from_le_bytes(e[21..29].try_into().unwrap()), len: u32::from_le_bytes(e[29..33].try_into().unwrap()) };
                entries.insert(key, Entry { source, image: OnceLock::new() });
            }
        }
        Ok(Pack { entries, file: Some(Mutex::new(f)) })
    }

    /// A folder or a data file, whichever the path is.
    pub(super) fn open(path: &Path) -> Result<Pack, String> {
        if path.is_dir() { Pack::from_dir(path) } else { Pack::from_file(path) }
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

fn decode_png(bytes: &[u8]) -> Result<Image, String> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
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
    /// Opens a pack: a folder of pictures named as `file_name` makes them, or a data file.
    pub(super) fn load_pack(&mut self, path: &Path) -> Result<(), String> {
        let pack = Pack::open(path)?;
        note(&format!("texture pack {}: {} textures", path.display(), pack.len()));
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

    fn key(data: u64, pal: u64, fmt: u8, w: u16, h: u16) -> TexKey {
        TexKey { data, pal, fmt, w, h }
    }

    #[test]
    fn a_data_file_gives_its_pictures_by_key() {
        // tests/data/tiny.spdata is made by tools/ps2/spdata.py (the same file the Python side reads)
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tiny.spdata");
        let pack = Pack::from_file(&path).expect("the fixture opens");
        assert_eq!(pack.len(), 2);
        let red = pack.get(&key(0x1111, 0x2222, 4, 8, 8)).expect("the red one");
        assert_eq!((red.w, red.h), (2, 2));
        assert_eq!(&red.rgba[..4], &[255, 0, 0, 255]);
        let blue = pack.get(&key(0x3333, 0, 7, 16, 8)).expect("the blue one");
        assert_eq!((blue.w, blue.h, &blue.rgba[..4]), (4, 2, &[0u8, 0, 255, 255][..]));
        assert!(pack.get(&key(1, 2, 3, 8, 8)).is_none());
        // the second ask is the same picture, decoded once
        assert!(Arc::ptr_eq(&red, &pack.get(&key(0x1111, 0x2222, 4, 8, 8)).unwrap()));
    }

    #[test]
    fn something_that_is_not_a_data_file_is_refused() {
        let dir = std::env::temp_dir().join(format!("sunpetal-spdata-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("bad.spdata");
        fs::write(&bad, b"not a data file at all").unwrap();
        assert!(Pack::from_file(&bad).is_err());
        let cut = dir.join("cut.spdata");
        let whole = fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tiny.spdata")).unwrap();
        fs::write(&cut, &whole[..60]).unwrap();
        assert!(Pack::from_file(&cut).is_err(), "an index cut short");
        // a newer version: the last magic byte
        let mut newer = whole.clone();
        newer[7] = 2;
        fs::write(&dir.join("newer.spdata"), &newer).unwrap();
        assert!(Pack::from_file(&dir.join("newer.spdata")).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_of_pictures_loads_lazily_and_open_takes_either() {
        let dir = std::env::temp_dir().join(format!("sunpetal-folder-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let k = key(0xAB, 0, 7, 8, 8);
        write_png(&dir.join(file_name(&k)), &Image { w: 1, h: 1, rgba: vec![9, 8, 7, 255] }).unwrap();
        fs::write(dir.join("notes.txt"), b"ignored").unwrap();
        let pack = Pack::open(&dir).unwrap();
        assert_eq!(pack.len(), 1);
        assert_eq!(pack.get(&k).unwrap().rgba, vec![9, 8, 7, 255]);
        let tiny = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tiny.spdata");
        assert_eq!(Pack::open(&tiny).unwrap().len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }
}
