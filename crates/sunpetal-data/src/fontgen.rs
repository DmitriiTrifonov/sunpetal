//! An NFTR font made from a TrueType/OpenType font for the characters of a translation
//! (docs/TRANSLATION-RECON.md, "NFTR"). The shape of the game's own font is kept: the cell, the
//! baseline, the depth in bits, the encoding. The user's font file is not part of the repository.
use std::collections::BTreeSet;

use ab_glyph::{Font as _, FontRef, PxScale, ScaleFont};

use crate::nftr::{CharMap, MapData, Nftr};

pub struct Params {
    /// The size in pixels (the height of the em).
    pub size: f32,
    /// Coverage gain: above 1 makes the strokes heavier, below 1 lighter.
    pub weight: f32,
    /// Pixel look, like the game's own hand-drawn font: the coverage (after the weight) is cut at
    /// `PIXEL_CUT` into full strokes, half of it into the middle level, and the advance is the ink
    /// (the game adds a pixel between letters).
    pub pixel: bool,
}

/// Coverage from which a pixel of a pixel-look glyph is full; half of it makes a middle one.
pub const PIXEL_CUT: f32 = 0.5;

impl Default for Params {
    fn default() -> Params {
        Params { size: 12.0, weight: 1.0, pixel: false }
    }
}

/// The text-box marker of the game's texts: the engine takes it as a code, the font keeps the star the game draws for it.
const MARKER: char = '\u{BD}';

/// Codes the font does not draw from the TTF: the marker (a star in the game's font) and the line ends.
const EMPTY: [char; 3] = [MARKER, '\r', '\n'];

/// The characters of the text that a font must have: printable ones, and the ASCII the game's own
/// texts and `?` need.
pub fn charset<'a>(texts: impl IntoIterator<Item = &'a str>) -> BTreeSet<char> {
    let mut set: BTreeSet<char> = (0x20u8..0x7F).map(char::from).collect();
    for t in texts {
        set.extend(t.chars().filter(|c| !c.is_control()));
    set.extend(EMPTY);
    }
    set
}

/// A font like `base` (the geometry, depth and encoding) with the glyphs of `ttf` for `chars`.
/// A character the file lacks is an error: better than a hole in the dialogue.
pub fn build(base: &Nftr, ttf: &[u8], chars: &BTreeSet<char>, p: &Params) -> Result<Nftr, String> {
    let font = FontRef::try_from_slice(ttf).map_err(|e| format!("the font file does not read: {}", e))?;
    let scaled = font.as_scaled(PxScale::from(p.size));
    let levels = ((1u32 << base.depth) - 1) as f32;
    let (cw, ch) = (base.cell_width as usize, base.cell_height as usize);
    let mut chars: Vec<char> = chars.iter().copied().collect();
    if !chars.contains(&'?') {
        chars.push('?');
    }
    if chars.len() > 0xFFFF {
        return Err("too many characters for one font".into());
    }
    let lacking: String = chars.iter().filter(|c| **c != ' ' && !EMPTY.contains(c) && font.glyph_id(**c).0 == 0).collect();
    if !lacking.is_empty() {
        return Err(format!("the font file lacks: {}", lacking));
    }
    let (mut glyphs, mut widths, mut pairs) = (Vec::new(), Vec::new(), Vec::new());
    for (i, &c) in chars.iter().enumerate() {
        if EMPTY.contains(&c) {
            // the marker is a star in the game's own font: keep that glyph; line ends stay empty
            let own = if c == MARKER { base_glyph(base, c) } else { None };
            widths.push(own.as_ref().map_or((0, 0, 0), |(_, w)| *w));
            glyphs.push(own.map_or_else(|| pack(&vec![0u8; cw * ch], base.depth, base.cell_size as usize), |(g, _)| g));
            pairs.push((c as u32 as u16, i as u16));
            continue;
        }
        let id = font.glyph_id(c);
        let adv = scaled.h_advance(id).round().clamp(0.0, 255.0) as u8;
        let mut cell = vec![0u8; cw * ch];
        let (mut bearing, mut ink) = (0i8, 0u8);
        let g = id.with_scale_and_position(PxScale::from(p.size), ab_glyph::point(0.0, base.baseline as f32));
        if let Some(o) = font.outline_glyph(g) {
            let b = o.px_bounds();
            let (x0, y0) = (b.min.x.floor() as i32, b.min.y.floor() as i32);
            bearing = x0.clamp(-128, 127) as i8;
            let mut right = 0usize;
            o.draw(|x, y, cov| {
                let (px, py) = (x as usize, (y0 + y as i32));
                if py < 0 || py as usize >= ch || px >= cw {
                    return;
                }
                let c = (cov * p.weight).clamp(0.0, 1.0);
                let v = if p.pixel {
                    if c >= PIXEL_CUT { levels } else if c >= PIXEL_CUT / 2.0 { (levels / 2.0).round() } else { 0.0 }
                } else {
                    c * levels
                };
                let v = v.round() as u8;
                if v > 0 {
                    cell[py as usize * cw + px] = v;
                    right = right.max(px + 1);
                }
            });
            ink = right as u8;
        }
        glyphs.push(pack(&cell, base.depth, base.cell_size as usize));
        let own = (bearing.max(0) as u8).saturating_add(ink);
        // pixel look: the letter's own width; the space keeps the font's advance
        let adv = if p.pixel && ink > 0 {
            own
        } else if c == ' ' {
            base_advance(base, ' ').unwrap_or(adv)
        } else {
            adv.max(own)
        };
        widths.push((bearing, ink, adv));
        pairs.push((c as u32 as u16, i as u16));
    }
    if chars.iter().any(|c| *c as u32 > 0xFFFF) {
        return Err("a character outside the first 65536 does not fit the font's 16-bit codes".into());
    }
    let alt = chars.iter().position(|c| *c == '?').unwrap() as u16;
    let mut out = base.clone();
    out.glyphs = glyphs;
    out.first_width = 0;
    out.widths = widths;
    out.alt_index = alt;
    out.maps = vec![CharMap { lo: 0, hi: 0xFFFF, data: MapData::Scan(pairs) }];
    Ok(out)
}

/// The advance the base font gives a character (the game reserves room for numbers in spaces).
fn base_advance(base: &Nftr, c: char) -> Option<u8> {
    base_glyph(base, c).map(|(_, w)| w.2)
}

/// The bitmap and widths the base font has for a character.
fn base_glyph(base: &Nftr, c: char) -> Option<(Vec<u8>, (i8, u8, u8))> {
    let code = c as u32 as u16;
    let g = base.maps.iter().find_map(|m| {
        if code < m.lo || code > m.hi {
            return None;
        }
        match &m.data {
            MapData::Direct(first) => Some(first + (code - m.lo)),
            MapData::Table(t) => t.get((code - m.lo) as usize).copied().filter(|g| *g != 0xFFFF),
            MapData::Scan(p) => p.iter().find(|(k, _)| *k == code).map(|(_, g)| *g),
        }
    })?;
    let w = *base.widths.get(g.checked_sub(base.first_width)? as usize)?;
    Some((base.glyphs.get(g as usize)?.clone(), w))
}

/// Pixels of a cell (one value each, row after row) as the font stores them: `depth` bits each,
/// most significant bit first, no padding between rows.
pub fn pack(cell: &[u8], depth: u8, size: usize) -> Vec<u8> {
    let mut out = vec![0u8; size];
    for (i, &v) in cell.iter().enumerate() {
        for b in 0..depth as usize {
            let bit = i * depth as usize + b;
            if bit / 8 < size && (v >> (depth as usize - 1 - b)) & 1 == 1 {
                out[bit / 8] |= 0x80 >> (bit % 8);
            }
        }
    }
    out
}

/// The reverse of `pack`.
pub fn unpack(bytes: &[u8], depth: u8, count: usize) -> Vec<u8> {
    (0..count)
        .map(|i| {
            (0..depth as usize).fold(0u8, |v, b| {
                let bit = i * depth as usize + b;
                (v << 1) | bytes.get(bit / 8).map_or(0, |x| (x >> (7 - bit % 8)) & 1)
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nftr::Font;

    pub(super) fn base() -> Nftr {
        Nftr {
            version: 0x100,
            font_type: 0,
            line_feed: 10,
            alt_index: 0,
            default_width: (0, 13, 13),
            encoding: 1,
            cell_width: 13,
            cell_height: 14,
            cell_size: 69,
            baseline: 11,
            max_width: 13,
            depth: 3,
            rotation: 0,
            glyphs: vec![],
            first_width: 0,
            widths: vec![],
            maps: vec![],
        }
    }

    const UBUNTU: &[u8] = epaint_default_fonts::UBUNTU_LIGHT;

    #[test]
    fn pixels_pack_and_unpack() {
        for depth in [1u8, 2, 3] {
            let cell: Vec<u8> = (0..13 * 14).map(|i| (i * 7 % (1 << depth)) as u8).collect();
            let size = (13 * 14 * depth as usize).div_ceil(8);
            assert_eq!(unpack(&pack(&cell, depth, size), depth, cell.len()), cell);
        }
    }

    #[test]
    fn a_made_font_survives_writing_and_has_the_letters() {
        let chars = charset(["Привет, мир", "Zażółć gęślą jaźń", "ΑΒΓ"]);
        let f = build(&base(), UBUNTU, &chars, &Params::default()).unwrap();
        let back = Nftr::parse(&f.write()).unwrap();
        assert_eq!(back, f);
        let m = Font::parse(&f.write()).unwrap();
        for c in "Привет, мир Zażółć gęślą jaźń ΑΒΓ".chars() {
            assert!(m.has(c), "{}", c);
        }
        // the letter has ink and a width; the space has neither ink nor a hole in the map
        let at = |c: char| chars.iter().position(|x| *x == c).unwrap();
        let (a, sp) = (f.widths[at('П')], f.widths[at(' ')]);
        assert!(a.1 > 3 && a.2 >= a.1, "{:?}", a);
        assert_eq!(sp.1, 0);
        assert!(sp.2 > 0);
        assert!(unpack(&f.glyphs[at('П')], 3, 13 * 14).iter().any(|v| *v > 0));
        assert!(m.dialogue_width("Привет") > m.dialogue_width("При"));
    }

    #[test]
    fn heavier_weight_darkens_and_a_missing_character_is_an_error() {
        let chars = charset(["О"]);
        let sum = |w: f32| {
            let f = build(&base(), UBUNTU, &chars, &Params { size: 12.0, weight: w, pixel: false }).unwrap();
            f.glyphs.iter().flat_map(|g| unpack(g, 3, 13 * 14)).map(|v| v as u32).sum::<u32>()
        };
        assert!(sum(1.6) > sum(0.8));
        let err = build(&base(), UBUNTU, &charset(["日"]), &Params::default()).unwrap_err();
        assert!(err.contains("日"), "{}", err);
    }
}

#[cfg(test)]
mod pixel_tests {
    use super::*;

    #[test]
    fn the_pixel_look_has_only_full_and_middle_levels_and_a_tight_advance() {
        let base = super::tests::base();
        let chars = charset(["Ш"]);
        let f = build(&base, epaint_default_fonts::UBUNTU_LIGHT, &chars, &Params { size: 13.0, weight: 1.6, pixel: true }).unwrap();
        let at = chars.iter().position(|c| *c == 'Ш').unwrap();
        let px = unpack(&f.glyphs[at], 3, 13 * 14);
        assert!(px.iter().all(|v| matches!(v, 0 | 4 | 7)), "{:?}", px.iter().collect::<std::collections::BTreeSet<_>>());
        let w = f.widths[at];
        assert_eq!(w.2, (w.0.max(0) as u8) + w.1);
    }
}
