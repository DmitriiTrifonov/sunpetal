//! The widths and the character map of an NFTR font (Nitro font), enough to measure text as the
//! game draws it. Not the glyph bitmaps.
use std::collections::HashMap;

pub struct Font {
    /// Per glyph: left bearing, ink width, advance.
    widths: Vec<(i8, u8, u8)>,
    map: HashMap<u32, usize>,
}

fn u16_at(d: &[u8], at: usize) -> Result<u16, String> {
    d.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or_else(|| "the font is cut short".to_string())
}

fn u32_at(d: &[u8], at: usize) -> Result<u32, String> {
    d.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).ok_or_else(|| "the font is cut short".to_string())
}

impl Font {
    pub fn parse(d: &[u8]) -> Result<Font, String> {
        if d.get(..4) != Some(b"RTFN".as_slice()) {
            return Err("not an NFTR font".into());
        }
        let (head, blocks) = (u16_at(d, 12)? as usize, u16_at(d, 14)? as usize);
        let (mut pos, mut finf, mut cwdh) = (head, None, None);
        for _ in 0..blocks {
            let size = u32_at(d, pos + 4)? as usize;
            match d.get(pos..pos + 4) {
                Some(b"FNIF") => finf = Some(pos),
                Some(b"HDWC") => cwdh = Some(pos),
                _ => {}
            }
            pos += size.max(8);
        }
        let (finf, cwdh) = (finf.ok_or("the font has no FINF")?, cwdh.ok_or("the font has no CWDH")?);
        let (first, last) = (u16_at(d, cwdh + 8)? as usize, u16_at(d, cwdh + 10)? as usize);
        let widths = (first..=last)
            .map(|i| {
                let at = cwdh + 16 + 3 * (i - first);
                d.get(at..at + 3).map(|b| (b[0] as i8, b[1], b[2])).ok_or_else(|| "the font is cut short".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        // the glyphs before `first` have no width entry: none are used
        let mut widths_all = vec![(0, 0, 0); first];
        widths_all.extend(widths);
        let mut map = HashMap::new();
        let mut off = u32_at(d, finf + 24)? as usize;
        while off != 0 {
            let b = off - 8;
            let (lo, hi, typ) = (u16_at(d, b + 8)? as u32, u16_at(d, b + 10)? as u32, u16_at(d, b + 12)?);
            let next = u32_at(d, b + 16)? as usize;
            match typ {
                0 => {
                    let idx = u16_at(d, b + 20)? as usize;
                    for c in lo..=hi {
                        map.insert(c, idx + (c - lo) as usize);
                    }
                }
                1 => {
                    for c in lo..=hi {
                        let v = u16_at(d, b + 20 + 2 * (c - lo) as usize)?;
                        if v != 0xFFFF {
                            map.insert(c, v as usize);
                        }
                    }
                }
                2 => {
                    for i in 0..u16_at(d, b + 20)? as usize {
                        map.insert(u16_at(d, b + 22 + 4 * i)? as u32, u16_at(d, b + 24 + 4 * i)? as usize);
                    }
                }
                _ => return Err(format!("character map of type {} is not understood", typ)),
            }
            off = next;
        }
        Ok(Font { widths: widths_all, map })
    }

    pub fn has(&self, c: char) -> bool {
        self.map.contains_key(&(c as u32))
    }

    fn glyph(&self, c: char) -> Option<(i8, u8, u8)> {
        let gi = self.map.get(&(c as u32)).or_else(|| self.map.get(&('?' as u32)))?;
        self.widths.get(*gi).copied()
    }

    /// The width in pixels of a line of dialogue as the game draws it: each character takes its
    /// advance and one pixel, the last one only its ink (measured in the game by the translation
    /// project).
    pub fn dialogue_width(&self, line: &str) -> usize {
        let n = line.chars().count();
        line.chars()
            .enumerate()
            .map(|(i, c)| match self.glyph(c) {
                Some((_, ink, adv)) => if i + 1 < n { adv as usize + 1 } else { ink as usize },
                None => 0,
            })
            .sum()
    }
}

/// One character map block (CMAP) of an NFTR font.
#[derive(Clone, PartialEq, Debug)]
pub enum MapData {
    /// Type 0: the codes `lo..=hi` take the glyphs `first..` in a row.
    Direct(u16),
    /// Type 1: one glyph per code, `0xFFFF` for none.
    Table(Vec<u16>),
    /// Type 2: (code, glyph) pairs; the range is `0..=0xFFFF`.
    Scan(Vec<(u16, u16)>),
}

#[derive(Clone, PartialEq, Debug)]
pub struct CharMap {
    pub lo: u16,
    pub hi: u16,
    pub data: MapData,
}

/// The whole NFTR font, glyph bitmaps included, to read and write back byte for byte
/// (docs/TRANSLATION-RECON.md, "NFTR"). Blocks come in the order FINF, CGLP, CWDH, CMAP...
#[derive(Clone, PartialEq, Debug)]
pub struct Nftr {
    /// File version: 0x0100 or 0x0101 (the same layout).
    pub version: u16,
    pub font_type: u8,
    pub line_feed: u8,
    /// The glyph for a character the font lacks.
    pub alt_index: u16,
    /// Left bearing, ink width, advance of the glyph that has no entry.
    pub default_width: (i8, u8, u8),
    /// 0 UTF-8, 1 UTF-16, 2 Shift-JIS, 3 CP1252.
    pub encoding: u8,
    pub cell_width: u8,
    pub cell_height: u8,
    /// Bytes per glyph bitmap.
    pub cell_size: u16,
    pub baseline: u8,
    pub max_width: u8,
    /// Bits per pixel.
    pub depth: u8,
    pub rotation: u8,
    /// `cell_size` bytes per glyph.
    pub glyphs: Vec<Vec<u8>>,
    /// The glyph the first width entry belongs to.
    pub first_width: u16,
    /// Left bearing, ink width, advance, from `first_width` on.
    pub widths: Vec<(i8, u8, u8)>,
    pub maps: Vec<CharMap>,
}

fn pad4(v: &mut Vec<u8>) {
    while v.len() % 4 != 0 {
        v.push(0);
    }
}

impl Nftr {
    pub fn parse(d: &[u8]) -> Result<Nftr, String> {
        if d.get(..4) != Some(b"RTFN".as_slice()) {
            return Err("not an NFTR font".into());
        }
        let (version, head, blocks) = (u16_at(d, 6)?, u16_at(d, 12)? as usize, u16_at(d, 14)? as usize);
        let (mut pos, mut finf, mut cglp, mut cwdh) = (head, None, None, None);
        for _ in 0..blocks {
            let size = u32_at(d, pos + 4)? as usize;
            match d.get(pos..pos + 4) {
                Some(b"FNIF") => finf = Some(pos),
                Some(b"PLGC") => cglp = Some(pos),
                Some(b"HDWC") => cwdh = Some(pos),
                _ => {}
            }
            pos += size.max(8);
        }
        let (finf, cglp, cwdh) = (finf.ok_or("the font has no FINF")?, cglp.ok_or("the font has no CGLP")?, cwdh.ok_or("the font has no CWDH")?);
        let byte = |at: usize| d.get(at).copied().ok_or_else(|| "the font is cut short".to_string());
        let cell_size = u16_at(d, cglp + 10)?;
        let glyph_bytes = (u32_at(d, cglp + 4)? as usize).saturating_sub(16);
        let count = if cell_size == 0 { 0 } else { glyph_bytes / cell_size as usize };
        let mut glyphs = Vec::with_capacity(count);
        for i in 0..count {
            let at = cglp + 16 + i * cell_size as usize;
            glyphs.push(d.get(at..at + cell_size as usize).ok_or("the font is cut short")?.to_vec());
        }
        let (first, last) = (u16_at(d, cwdh + 8)?, u16_at(d, cwdh + 10)?);
        let widths = (first as usize..=last as usize)
            .map(|i| {
                let at = cwdh + 16 + 3 * (i - first as usize);
                d.get(at..at + 3).map(|b| (b[0] as i8, b[1], b[2])).ok_or_else(|| "the font is cut short".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut maps = Vec::new();
        let mut off = u32_at(d, finf + 24)? as usize;
        while off != 0 {
            let b = off.checked_sub(8).ok_or("bad character map pointer")?;
            let (lo, hi, typ) = (u16_at(d, b + 8)?, u16_at(d, b + 10)?, u16_at(d, b + 12)?);
            let next = u32_at(d, b + 16)? as usize;
            let data = match typ {
                0 => MapData::Direct(u16_at(d, b + 20)?),
                1 => MapData::Table((0..=(hi - lo) as usize).map(|i| u16_at(d, b + 20 + 2 * i)).collect::<Result<_, _>>()?),
                2 => MapData::Scan((0..u16_at(d, b + 20)? as usize).map(|i| Ok((u16_at(d, b + 22 + 4 * i)?, u16_at(d, b + 24 + 4 * i)?))).collect::<Result<_, String>>()?),
                _ => return Err(format!("character map of type {} is not understood", typ)),
            };
            maps.push(CharMap { lo, hi, data });
            off = next;
        }
        Ok(Nftr {
            version,
            font_type: byte(finf + 8)?,
            line_feed: byte(finf + 9)?,
            alt_index: u16_at(d, finf + 10)?,
            default_width: (byte(finf + 12)? as i8, byte(finf + 13)?, byte(finf + 14)?),
            encoding: byte(finf + 15)?,
            cell_width: byte(cglp + 8)?,
            cell_height: byte(cglp + 9)?,
            cell_size,
            baseline: byte(cglp + 12)?,
            max_width: byte(cglp + 13)?,
            depth: byte(cglp + 14)?,
            rotation: byte(cglp + 15)?,
            glyphs,
            first_width: first,
            widths,
            maps,
        })
    }

    pub fn write(&self) -> Vec<u8> {
        fn block(magic: &[u8; 4], body: &[u8]) -> Vec<u8> {
            let mut v = magic.to_vec();
            v.extend(((body.len() + 8) as u32).to_le_bytes());
            v.extend(body);
            v
        }
        let mut cglp = vec![self.cell_width, self.cell_height];
        cglp.extend(self.cell_size.to_le_bytes());
        cglp.extend([self.baseline, self.max_width, self.depth, self.rotation]);
        for g in &self.glyphs {
            cglp.extend(g);
        }
        pad4(&mut cglp);
        // the width block has a 8-byte header of its own after the common 8
        let mut cwdh = self.first_width.to_le_bytes().to_vec();
        cwdh.extend((self.first_width + self.widths.len().max(1) as u16 - 1).to_le_bytes());
        cwdh.extend(0u32.to_le_bytes());
        for w in &self.widths {
            cwdh.extend([w.0 as u8, w.1, w.2]);
        }
        pad4(&mut cwdh);
        let (finf_len, cglp_len, cwdh_len) = (28usize, cglp.len() + 8, cwdh.len() + 8);
        let mut at = 16 + finf_len + cglp_len + cwdh_len;
        let mut cmaps = Vec::new();
        for (i, m) in self.maps.iter().enumerate() {
            let mut body = m.lo.to_le_bytes().to_vec();
            body.extend(m.hi.to_le_bytes());
            let (typ, tail): (u16, Vec<u8>) = match &m.data {
                MapData::Direct(first) => (0, first.to_le_bytes().to_vec()),
                MapData::Table(t) => (1, t.iter().flat_map(|v| v.to_le_bytes()).collect()),
                MapData::Scan(p) => (2, (p.len() as u16).to_le_bytes().into_iter().chain(p.iter().flat_map(|(c, g)| c.to_le_bytes().into_iter().chain(g.to_le_bytes()))).collect()),
            };
            body.extend(typ.to_le_bytes());
            body.extend(0u16.to_le_bytes());
            let mut tail = tail;
            pad4(&mut tail);
            let len = 8 + 12 + tail.len();
            let next = if i + 1 < self.maps.len() { (at + len + 8) as u32 } else { 0 };
            body.extend(next.to_le_bytes());
            body.extend(tail);
            at += len;
            cmaps.push(block(b"PAMC", &body));
        }
        let first_map = if self.maps.is_empty() { 0 } else { (16 + finf_len + cglp_len + cwdh_len + 8) as u32 };
        let mut finf = vec![self.font_type, self.line_feed];
        finf.extend(self.alt_index.to_le_bytes());
        finf.extend([self.default_width.0 as u8, self.default_width.1, self.default_width.2, self.encoding]);
        finf.extend(((16 + finf_len + 8) as u32).to_le_bytes());
        finf.extend(((16 + finf_len + cglp_len + 8) as u32).to_le_bytes());
        finf.extend(first_map.to_le_bytes());
        let mut out = b"RTFN".to_vec();
        out.extend(0xFEFFu16.to_le_bytes());
        out.extend(self.version.to_le_bytes());
        out.extend([0; 4]);
        out.extend(16u16.to_le_bytes());
        out.extend((3 + cmaps.len() as u16).to_le_bytes());
        out.extend(block(b"FNIF", &finf));
        out.extend(block(b"PLGC", &cglp));
        out.extend(block(b"HDWC", &cwdh));
        for c in cmaps {
            out.extend(c);
        }
        let n = out.len() as u32;
        out[8..12].copy_from_slice(&n.to_le_bytes());
        out
    }
}
