//! A binary delta of a file against its original: what a translation or a texture pack holds for a
//! file that is mostly the original's (a font with Cyrillic added, a page of the guide with the text
//! redrawn). The delta carries the bytes that are new and where to copy the rest from, so by itself
//! it is not the original; it applies only to the file it was made against (length and CRC-32 of
//! both are in the header).
//!
//! ```text
//! "SPD1"  varint orig_len  u32 orig_crc  varint new_len  u32 new_crc  ops...
//! op: varint n<<1      n bytes follow (new data)
//!     varint n<<1 | 1  copy n bytes from the original at (the place the last copy's shift predicts
//!                      + a zigzag varint)
//! ```
//! The shift is `original offset - new offset` of the last copy: an edit that keeps the layout
//! (pixels redrawn in place) copies with a shift of 0 and costs 2 bytes per run.
use std::collections::HashMap;

const MAGIC: &[u8; 4] = b"SPD1";
/// The shortest copy worth its bytes: at the predicted place, and elsewhere.
const MIN_NEAR: usize = 4;
const MIN_FAR: usize = 12;
const KEY: usize = 8;
const CANDIDATES: usize = 32;

pub fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *t = c;
    }
    !data.iter().fold(!0u32, |c, b| table[((c ^ *b as u32) & 0xFF) as usize] ^ (c >> 8))
}

pub(crate) fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v & 0x7F) as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

pub(crate) fn get_varint(data: &[u8], at: &mut usize) -> Result<u64, String> {
    let (mut v, mut shift) = (0u64, 0);
    loop {
        let b = *data.get(*at).ok_or("the delta ends early")?;
        *at += 1;
        v |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            return Ok(v);
        }
        shift += 7;
        if shift > 63 {
            return Err("a number in the delta is too long".into());
        }
    }
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

fn unzigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

fn match_len(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

fn key_at(data: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(data[at..at + KEY].try_into().unwrap())
}

pub fn diff(orig: &[u8], new: &[u8]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    put_varint(&mut out, orig.len() as u64);
    out.extend_from_slice(&crc32(orig).to_le_bytes());
    put_varint(&mut out, new.len() as u64);
    out.extend_from_slice(&crc32(new).to_le_bytes());

    let mut index: HashMap<u64, Vec<u32>> = HashMap::new();
    for at in 0..orig.len().saturating_sub(KEY - 1) {
        let v = index.entry(key_at(orig, at)).or_default();
        if v.len() < CANDIDATES {
            v.push(at as u32);
        }
    }
    let (mut pos, mut lit, mut shift) = (0usize, 0usize, 0i64);
    let flush = |out: &mut Vec<u8>, from: usize, to: usize| {
        if to > from {
            put_varint(out, ((to - from) as u64) << 1);
            out.extend_from_slice(&new[from..to]);
        }
    };
    while pos < new.len() {
        let mut best: Option<(usize, usize)> = None; // (original offset, length)
        let near = pos as i64 + shift;
        if near >= 0 && (near as usize) < orig.len() {
            let l = match_len(&orig[near as usize..], &new[pos..]);
            if l >= MIN_NEAR {
                best = Some((near as usize, l));
            }
        }
        if pos + KEY <= new.len() {
            if let Some(cands) = index.get(&key_at(new, pos)) {
                for &c in cands {
                    let l = match_len(&orig[c as usize..], &new[pos..]);
                    if l >= MIN_FAR && best.map_or(true, |(_, bl)| l > bl) {
                        best = Some((c as usize, l));
                    }
                }
            }
        }
        let Some((mut o, mut len)) = best else {
            pos += 1;
            continue;
        };
        // the pending new bytes may be the end of this match too
        while pos > lit && o > 0 && orig[o - 1] == new[pos - 1] {
            pos -= 1;
            o -= 1;
            len += 1;
        }
        flush(&mut out, lit, pos);
        put_varint(&mut out, (len as u64) << 1 | 1);
        put_varint(&mut out, zigzag(o as i64 - (pos as i64 + shift)));
        shift = o as i64 - pos as i64;
        pos += len;
        lit = pos;
    }
    flush(&mut out, lit, new.len());
    out
}

/// The delta for a file of the ROM: the data of a `.z` file (LZ10) is compared unpacked.
pub fn diff_file(path: &str, orig: &[u8], new: &[u8]) -> Result<Vec<u8>, String> {
    if path.ends_with(".z") {
        Ok(diff(&crate::lz10::decompress(orig)?, &crate::lz10::decompress(new)?))
    } else {
        Ok(diff(orig, new))
    }
}

/// The file from its original and the delta made by `diff_file`; a `.z` file is packed again.
pub fn patch_file(path: &str, orig: &[u8], delta: &[u8]) -> Result<Vec<u8>, String> {
    if path.ends_with(".z") {
        Ok(crate::lz10::compress(&patch(&crate::lz10::decompress(orig)?, delta)?))
    } else {
        patch(orig, delta)
    }
}

/// What the delta was made against: (length, CRC-32) of the original and of the result.
pub fn header(delta: &[u8]) -> Result<((usize, u32), (usize, u32)), String> {
    parse_header(delta).map(|(o, n, _)| (o, n))
}

fn parse_header(delta: &[u8]) -> Result<((usize, u32), (usize, u32), usize), String> {
    if delta.get(..4) != Some(MAGIC) {
        return Err("not a delta".into());
    }
    let mut at = 4;
    let crc = |at: &mut usize| -> Result<u32, String> {
        let b = delta.get(*at..*at + 4).ok_or("the delta ends early")?;
        *at += 4;
        Ok(u32::from_le_bytes(b.try_into().unwrap()))
    };
    let ol = get_varint(delta, &mut at)? as usize;
    let oc = crc(&mut at)?;
    let nl = get_varint(delta, &mut at)? as usize;
    let nc = crc(&mut at)?;
    Ok(((ol, oc), (nl, nc), at))
}

/// The file the delta was made for, from its original; an error if the original is not the one.
pub fn patch(orig: &[u8], delta: &[u8]) -> Result<Vec<u8>, String> {
    let ((ol, oc), (nl, nc), mut at) = parse_header(delta)?;
    if orig.len() != ol || crc32(orig) != oc {
        return Err("the delta is for another original".into());
    }
    let mut out = Vec::with_capacity(nl);
    let mut shift = 0i64;
    while at < delta.len() {
        let op = get_varint(delta, &mut at)?;
        let n = (op >> 1) as usize;
        if out.len() + n > nl {
            return Err("the delta makes more than it says".into());
        }
        if op & 1 == 0 {
            out.extend_from_slice(delta.get(at..at + n).ok_or("the delta ends early")?);
            at += n;
        } else {
            let o = out.len() as i64 + shift + unzigzag(get_varint(delta, &mut at)?);
            let o = usize::try_from(o).map_err(|_| "a copy from before the original")?;
            out.extend_from_slice(orig.get(o..o + n).ok_or("a copy past the original")?);
            shift = o as i64 - (out.len() - n) as i64;
        }
    }
    if out.len() != nl || crc32(&out) != nc {
        return Err("the delta did not make the file it was made for".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, seed: u32) -> Vec<u8> {
        let mut s = seed;
        (0..n).map(|_| { s = s.wrapping_mul(1664525).wrapping_add(1013904223); (s >> 24) as u8 }).collect()
    }

    #[test]
    fn crc32_of_the_check_string() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn an_edit_in_place_costs_little() {
        let orig = noise(50_000, 1);
        let mut new = orig.clone();
        new[1000..1100].copy_from_slice(&noise(100, 2));
        new[30_000..30_050].copy_from_slice(&noise(50, 3));
        let d = diff(&orig, &new);
        assert!(d.len() < 300, "{} bytes", d.len());
        assert_eq!(patch(&orig, &d).unwrap(), new);
    }

    #[test]
    fn it_follows_data_that_moved_and_data_that_grew() {
        let orig = noise(20_000, 4);
        let mut new = orig[..5000].to_vec();
        new.extend(noise(300, 5));
        new.extend_from_slice(&orig[5000..15_000]);
        new.extend_from_slice(&orig[2000..4000]); // copied from earlier
        let d = diff(&orig, &new);
        assert!(d.len() < 450, "{} bytes", d.len());
        assert_eq!(patch(&orig, &d).unwrap(), new);
    }

    #[test]
    fn it_takes_the_empty_and_the_unlike() {
        for (a, b) in [(vec![], vec![]), (vec![], noise(100, 6)), (noise(100, 7), vec![]), (noise(500, 8), noise(500, 9))] {
            assert_eq!(patch(&a, &diff(&a, &b)).unwrap(), b);
        }
    }

    #[test]
    fn another_original_is_refused() {
        let a = noise(1000, 10);
        let d = diff(&a, &noise(1000, 11));
        let mut other = a.clone();
        other[5] ^= 1;
        assert!(patch(&other, &d).is_err());
        assert!(patch(&a, &d[..d.len() - 3]).is_err());
        assert!(patch(&a, b"nope").is_err());
    }
}
