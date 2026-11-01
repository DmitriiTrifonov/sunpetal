//! 2D packs (stage 8е.3, `docs/2D-UPSCALE.md`): a folder of PNG pictures named as `dump_2d` names the
//! sprites (`obj{engine}_{w}x{h}_{data}_{pal}.png`; the engine is left out of the key, so a picture
//! serves either screen). A sprite whose key is in the pack is not drawn by the 2D engine; its
//! picture, of any size (usually the original's times a whole number), is laid over the screen
//! at the picture's resolution (`Nds::overlay_pack`). Without a pack nothing changes.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// A sprite's key: its size in pixels and the hashes of its data and of its palette.
pub(super) type Key = (i32, i32, u64, u64);

pub(super) struct Img {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

struct Entry {
    path: PathBuf,
    image: OnceLock<Option<Arc<Img>>>,
}

#[derive(Default)]
pub(super) struct Pack2d {
    entries: HashMap<Key, Entry>,
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
    pub fn load(dir: &Path) -> Result<Pack2d, String> {
        let mut entries = HashMap::new();
        let mut layers = HashMap::new();
        let rd = std::fs::read_dir(dir).map_err(|e| format!("2D pack {}: {}", dir.display(), e))?;
        for f in rd.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            if let Some(k) = parse_name(&name) {
                entries.insert(k, Entry { path: f.path(), image: OnceLock::new() });
            } else if let Some(h) = parse_layer_name(&name) {
                layers.insert(h, Entry { path: f.path(), image: OnceLock::new() });
            }
        }
        Ok(Pack2d { entries, layers })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.layers.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len() + self.layers.len()
    }

    pub fn has_layers(&self) -> bool {
        !self.layers.is_empty()
    }

    /// The picture of a background layer whose pixels hash to `h`.
    pub fn get_layer(&self, h: u64) -> Option<Arc<Img>> {
        let e = self.layers.get(&h)?;
        e.image.get_or_init(|| read_png(&e.path).map(Arc::new)).clone()
    }

    /// The picture of `key`, decoded the first time it is asked for; `None` if there is none or it does not read.
    pub fn get(&self, key: &Key) -> Option<Arc<Img>> {
        let e = self.entries.get(key)?;
        e.image.get_or_init(|| read_png(&e.path).map(Arc::new)).clone()
    }
}

fn read_png(path: &Path) -> Option<Img> {
    let mut dec = png::Decoder::new(std::fs::File::open(path).ok()?);
    dec.set_transformations(png::Transformations::EXPAND);
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
    (w > 0 && h > 0).then_some(Img { w, h, rgba })
}

impl Img {
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

    #[test]
    fn a_picture_samples_to_its_own_pixels_at_its_own_size() {
        let img = Img { w: 2, h: 1, rgba: vec![255, 0, 0, 255, 0, 0, 255, 0] };
        assert_eq!(img.sample(0.25, 0.5), [255, 0, 0, 255]);
        assert_eq!(img.sample(0.75, 0.5)[3], 0);
    }
}
