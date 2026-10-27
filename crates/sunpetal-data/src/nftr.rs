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
