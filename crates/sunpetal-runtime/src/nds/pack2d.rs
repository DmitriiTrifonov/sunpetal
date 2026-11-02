//! 2D packs (stage 8е.3, `docs/2D-UPSCALE.md`): a folder of PNG pictures named as `dump_2d` names the
//! sprites (`obj{engine}_{w}x{h}_{data}_{pal}.png`; the engine is left out of the key, so a picture
//! serves either screen). A sprite whose key is in the pack is not drawn by the 2D engine; its
//! picture, of any size (usually the original's times a whole number), is laid over the screen
//! at the picture's resolution (`Nds::overlay_pack`). Without a pack nothing changes.
//!
//! A picture may be one of a group's variants: `obj0_16x32_{data}_{pal}@digits.rubik.png` is the variant `rubik`
//! of the group `digits`. The variant chosen for the group (`set_variant`) is drawn; `original` draws the game's
//! own sprite; with none chosen, the picture without a variant, else the group's first variant by name.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// A sprite's key: its size in pixels and the hashes of its data and of its palette.
pub(super) type Key = (i32, i32, u64, u64);

pub(super) struct Img {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
    /// The picture sampled to one size (a whole layer at the window's scale), kept from frame to frame.
    scaled: std::sync::Mutex<Option<(usize, usize, Arc<Vec<u8>>)>>,
}

struct Entry {
    source: Source,
    image: OnceLock<Option<Arc<Img>>>,
}

enum Source {
    Path(PathBuf),
    Bytes(Vec<u8>),
}

impl Entry {
    fn picture(&self) -> Option<Arc<Img>> {
        self.image
            .get_or_init(|| match &self.source {
                Source::Path(p) => std::fs::read(p).ok().and_then(|b| decode(&b)),
                Source::Bytes(b) => decode(b),
            }
            .map(Arc::new))
            .clone()
    }
}

/// The variant chosen for each group (`set_variant`), for every pack.
static CHOSEN: std::sync::RwLock<BTreeMap<String, String>> = std::sync::RwLock::new(BTreeMap::new());

/// The variant drawn for `group` from now on (`original`: the game's own sprites).
pub fn set_variant(group: &str, variant: &str) {
    CHOSEN.write().unwrap().insert(group.to_owned(), variant.to_owned());
}

/// The variant chosen for `group`, if one was.
pub fn chosen_variant(group: &str) -> Option<String> {
    CHOSEN.read().unwrap().get(group).cloned()
}

/// A variant of a sprite's picture: its group, its name.
struct Variant {
    group: String,
    name: String,
    entry: Entry,
}

#[derive(Default)]
pub(super) struct Pack2d {
    entries: HashMap<Key, Entry>,
    /// Pictures that are variants of a group, by key, sorted by variant name.
    variants: HashMap<Key, Vec<Variant>>,
    /// Background layers, by the hash of their pixels.
    layers: HashMap<u64, Entry>,
}

/// `bg01_{16 hex}.png` -> the hash of the layer's pixels.
pub(super) fn parse_layer_name(name: &str) -> Option<u64> {
    let stem = name.strip_suffix(".png")?.strip_prefix("bg")?;
    let (el, h) = stem.split_once('_')?;
    if el.len() != 2 || !el.bytes().all(|b| b.is_ascii_digit()) || h.len() != 16 {
        return None;
    }
    u64::from_str_radix(h, 16).ok()
}

/// `obj0_32x16_{16 hex}_{16 hex}@group.variant.png` -> the key, the group and the variant.
pub(super) fn parse_variant_name(name: &str) -> Option<(Key, String, String)> {
    let stem = name.strip_suffix(".png")?;
    let (base, v) = stem.split_once('@')?;
    let (group, variant) = v.split_once('.')?;
    if group.is_empty() || variant.is_empty() || variant.contains('.') {
        return None;
    }
    Some((parse_name(&format!("{}.png", base))?, group.to_owned(), variant.to_owned()))
}

/// `obj0_32x16_{16 hex}_{16 hex}.png` -> its key.
pub(super) fn parse_name(name: &str) -> Option<Key> {
    let stem = name.strip_suffix(".png")?.strip_prefix("obj")?;
    let (_engine, rest) = stem.split_once('_')?;
    let mut it = rest.split('_');
    let (wh, d, p) = (it.next()?, it.next()?, it.next()?);
    if it.next().is_some() {
        return None;
    }
    let (w, h) = wh.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?, u64::from_str_radix(d, 16).ok()?, u64::from_str_radix(p, 16).ok()?))
}

impl Pack2d {
    /// A folder of PNGs, or a data file with a 2D part (`sunpetal-pack-2d`).
    pub fn load(dir: &Path) -> Result<Pack2d, String> {
        let mut p = Pack2d::default();
        if dir.is_file() {
            let c = sunpetal_data::container::Container::read_only(dir, |t| t == sunpetal_data::pack2d::TAG)?;
            for s in c.all(sunpetal_data::pack2d::TAG) {
                for (name, png) in sunpetal_data::pack2d::unpack(&s.data)? {
                    p.add(&name, Source::Bytes(png));
                }
            }
        } else {
            let rd = std::fs::read_dir(dir).map_err(|e| format!("2D pack {}: {}", dir.display(), e))?;
            for f in rd.flatten() {
                p.add(&f.file_name().to_string_lossy(), Source::Path(f.path()));
            }
        }
        for v in p.variants.values_mut() {
            v.sort_by(|a, b| (&a.group, &a.name).cmp(&(&b.group, &b.name)));
        }
        Ok(p)
    }

    fn add(&mut self, name: &str, source: Source) {
        let entry = Entry { source, image: OnceLock::new() };
        if let Some((k, group, name)) = parse_variant_name(name) {
            self.variants.entry(k).or_default().push(Variant { group, name, entry });
        } else if let Some(k) = parse_name(name) {
            self.entries.insert(k, entry);
        } else if let Some(h) = parse_layer_name(name) {
            self.layers.insert(h, entry);
        }
    }

    /// The groups of variants and the variants of each.
    pub fn groups(&self) -> BTreeMap<String, BTreeSet<String>> {
        let mut g: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for v in self.variants.values().flatten() {
            g.entry(v.group.clone()).or_default().insert(v.name.clone());
        }
        g
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.layers.is_empty() && self.variants.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len() + self.layers.len() + self.variants.values().map(Vec::len).sum::<usize>()
    }

    pub fn has_layers(&self) -> bool {
        !self.layers.is_empty()
    }

    /// The picture of a background layer whose pixels hash to `h`.
    pub fn get_layer(&self, h: u64) -> Option<Arc<Img>> {
        self.layers.get(&h)?.picture()
    }

    /// The picture of `key`, decoded the first time it is asked for; `None` if there is none or it does not read.
    pub fn get(&self, key: &Key) -> Option<Arc<Img>> {
        if let Some(vs) = self.variants.get(key) {
            let chosen = chosen_variant(&vs[0].group);
            match chosen.as_deref() {
                Some("original") => return None,
                Some(c) => {
                    if let Some(v) = vs.iter().find(|v| v.name == c) {
                        return v.entry.picture();
                    }
                }
                None => {}
            }
            return match self.entries.get(key) {
                Some(e) => e.picture(),
                None => vs[0].entry.picture(),
            };
        }
        self.entries.get(key)?.picture()
    }
}

fn decode(bytes: &[u8]) -> Option<Img> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0u8; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match (info.color_type, info.bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Eight) => buf[..w * h * 4].to_vec(),
        (png::ColorType::Rgb, png::BitDepth::Eight) => buf[..w * h * 3].chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        (png::ColorType::GrayscaleAlpha, png::BitDepth::Eight) => buf[..w * h * 2].chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        (png::ColorType::Grayscale, png::BitDepth::Eight) => buf[..w * h].iter().flat_map(|&g| [g, g, g, 255]).collect(),
        _ => return None,
    };
    (w > 0 && h > 0).then_some(Img { w, h, rgba, scaled: Default::default() })
}

impl Img {
    /// The picture stretched over `w` x `h` pixels, as `sample` gives it at each pixel's middle (RGBA bytes);
    /// made once for a size, not on every frame.
    pub fn scaled(&self, w: usize, h: usize) -> Arc<Vec<u8>> {
        let mut c = self.scaled.lock().unwrap();
        if let Some((cw, ch, v)) = &*c {
            if (*cw, *ch) == (w, h) {
                return v.clone();
            }
        }
        let mut v = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                v.extend_from_slice(&self.sample((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32));
            }
        }
        let v = Arc::new(v);
        *c = Some((w, h, v.clone()));
        v
    }

    /// The pixel at (`x`, `y`), (r, g, b, a) bytes.
    pub fn texel(&self, x: usize, y: usize) -> [u8; 4] {
        let o = (y.min(self.h - 1) * self.w + x.min(self.w - 1)) * 4;
        [self.rgba[o], self.rgba[o + 1], self.rgba[o + 2], self.rgba[o + 3]]
    }

    /// The picture at (`fx`, `fy`) in 0..1 across and down, bilinear, alpha premultiplied while it
    /// is mixed; (r, g, b, a) bytes.
    pub fn sample(&self, fx: f32, fy: f32) -> [u8; 4] {
        let x = (fx * self.w as f32 - 0.5).clamp(0.0, (self.w - 1) as f32);
        let y = (fy * self.h as f32 - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (x0, y0) = (x as usize, y as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (tx, ty) = (x - x0 as f32, y - y0 as f32);
        let px = |xx: usize, yy: usize| {
            let o = (yy * self.w + xx) * 4;
            let a = self.rgba[o + 3] as f32 / 255.0;
            [self.rgba[o] as f32 * a, self.rgba[o + 1] as f32 * a, self.rgba[o + 2] as f32 * a, a]
        };
        let (p00, p10, p01, p11) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
        let mut m = [0f32; 4];
        for k in 0..4 {
            m[k] = (p00[k] * (1.0 - tx) + p10[k] * tx) * (1.0 - ty) + (p01[k] * (1.0 - tx) + p11[k] * tx) * ty;
        }
        if m[3] <= 0.0 {
            return [0; 4];
        }
        [(m[0] / m[3]).round() as u8, (m[1] / m[3]).round() as u8, (m[2] / m[3]).round() as u8, (m[3] * 255.0).round() as u8]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dump_name_gives_its_key() {
        assert_eq!(parse_name("obj1_32x16_00000000000000ab_00000000000000cd.png"), Some((32, 16, 0xab, 0xcd)));
        assert_eq!(parse_name("obj0_8x32_ffffffffffffffff_0000000000000001.png"), Some((8, 32, u64::MAX, 1)));
        assert_eq!(parse_name("bg00_00000000000000ab.png"), None);
        assert_eq!(parse_layer_name("bg13_00000000000000ab.png"), Some(0xab));
        assert_eq!(parse_layer_name("bg1_00000000000000ab.png"), None);
        assert_eq!(parse_layer_name("obj0_8x8_00000000000000ab.png"), None);
        assert_eq!(parse_name("obj0_32x16_ab_cd_ef.png"), None);
        assert_eq!(parse_name("obj0_32x16_ab_cd.jpg"), None);
    }

    fn png(rgba: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, 1, 1);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&rgba).unwrap();
        out
    }

    #[test]
    fn a_group_draws_the_variant_chosen_for_it() {
        assert_eq!(parse_variant_name("obj0_8x8_00000000000000ab_00000000000000cd@dig.rubik.png"), Some(((8, 8, 0xab, 0xcd), "dig".into(), "rubik".into())));
        assert_eq!(parse_variant_name("obj0_8x8_00000000000000ab_00000000000000cd.png"), None);
        assert_eq!(parse_variant_name("obj0_8x8_00000000000000ab_00000000000000cd@dig.png"), None);
        let base = "obj0_8x8_00000000000000ab_00000000000000cd";
        let mut p = Pack2d::default();
        // a test-only group, so that the global choice does not touch other tests
        p.add(&format!("{}@tgroup.b.png", base), Source::Bytes(png([0, 0, 255, 255])));
        p.add(&format!("{}@tgroup.a.png", base), Source::Bytes(png([255, 0, 0, 255])));
        for v in p.variants.values_mut() {
            v.sort_by(|a, b| (&a.group, &a.name).cmp(&(&b.group, &b.name)));
        }
        let key = (8, 8, 0xab, 0xcd);
        let red = |p: &Pack2d| p.get(&key).map(|i| i.rgba[0]);
        assert_eq!(p.groups()["tgroup"].iter().map(String::as_str).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(red(&p), Some(255), "none chosen: the first variant by name");
        set_variant("tgroup", "b");
        assert_eq!(red(&p), Some(0));
        set_variant("tgroup", "original");
        assert!(p.get(&key).is_none(), "original: the game's own sprite");
        set_variant("tgroup", "gone");
        assert_eq!(red(&p), Some(255), "an unknown variant: as with none chosen");
        p.add(&format!("{}.png", base), Source::Bytes(png([7, 7, 7, 255])));
        assert_eq!(red(&p), Some(7), "the picture without a variant is the default");
    }

    #[test]
    fn a_picture_samples_to_its_own_pixels_at_its_own_size() {
        let img = Img { w: 2, h: 1, rgba: vec![255, 0, 0, 255, 0, 0, 255, 0], scaled: Default::default() };
        assert_eq!(img.sample(0.25, 0.5), [255, 0, 0, 255]);
        assert_eq!(img.sample(0.75, 0.5)[3], 0);
    }
}
