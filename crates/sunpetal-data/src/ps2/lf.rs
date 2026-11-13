//! `LF` archives of the PS2 disc (У1.2): a header of `count` (size, id) pairs, then the records, each
//! aligned to 0x800. A record's `id` is the sector where the next record starts (F-111, checked by
//! `ids_follow_sectors`); the last record's `id` is 0.
use std::ops::Range;

pub const ALIGN: usize = 0x800;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    /// Size without the alignment.
    pub size: u32,
    pub id: u32,
    /// Offset of the record in the file.
    pub at: usize,
}

impl Record {
    pub fn range(&self) -> Range<usize> {
        self.at..self.at + self.size as usize
    }
}

#[derive(Debug)]
pub struct Archive {
    pub version: u32,
    pub records: Vec<Record>,
    /// Bytes after the last record's alignment.
    pub tail: usize,
}

fn aligned(n: usize) -> usize {
    n.div_ceil(ALIGN) * ALIGN
}

/// Reads the header. An error when the bytes are not an `LF` archive or the records run past the end.
pub fn parse(raw: &[u8]) -> Result<Archive, String> {
    if raw.len() < 8 || &raw[..2] != b"LF" {
        return Err("not an LF archive".into());
    }
    let count = u16::from_le_bytes([raw[2], raw[3]]) as usize;
    let version = u32::from_le_bytes(raw[4..8].try_into().unwrap());
    let mut at = ALIGN;
    let mut records = Vec::with_capacity(count);
    for i in 0..count {
        let h = raw.get(8 + 8 * i..16 + 8 * i).ok_or("header runs past the end")?;
        let size = u32::from_le_bytes(h[..4].try_into().unwrap());
        let id = u32::from_le_bytes(h[4..].try_into().unwrap());
        if at + size as usize > raw.len() {
            return Err(format!("record {i} runs past the end"));
        }
        records.push(Record { size, id, at });
        at += aligned(size as usize);
    }
    Ok(Archive { version, records, tail: raw.len().saturating_sub(at) })
}

impl Archive {
    pub fn data<'a>(&self, raw: &'a [u8], i: usize) -> Option<&'a [u8]> {
        raw.get(self.records.get(i)?.range())
    }

    /// Whether every record's `id` is the sector of the next record's start and the last one's is 0.
    pub fn ids_follow_sectors(&self) -> bool {
        let n = self.records.len();
        self.records.iter().enumerate().all(|(i, r)| if i + 1 < n { r.id as usize * ALIGN == self.records[i + 1].at } else { r.id == 0 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(sizes: &[u32], ids: &[u32]) -> Vec<u8> {
        let mut raw = vec![0u8; ALIGN];
        raw[..2].copy_from_slice(b"LF");
        raw[2..4].copy_from_slice(&(sizes.len() as u16).to_le_bytes());
        raw[4..8].copy_from_slice(&1u32.to_le_bytes());
        for (i, (s, id)) in sizes.iter().zip(ids).enumerate() {
            raw[8 + 8 * i..12 + 8 * i].copy_from_slice(&s.to_le_bytes());
            raw[12 + 8 * i..16 + 8 * i].copy_from_slice(&id.to_le_bytes());
        }
        for (i, s) in sizes.iter().enumerate() {
            raw.extend(std::iter::repeat_n(i as u8 + 1, aligned(*s as usize)));
        }
        raw
    }

    #[test]
    fn records_are_cut_at_their_sizes_and_found_at_the_alignment() {
        let raw = archive(&[16, 0x800, 0x801], &[2, 3, 0]);
        let a = parse(&raw).unwrap();
        assert_eq!(a.records.iter().map(|r| r.at).collect::<Vec<_>>(), [0x800, 0x1000, 0x1800]);
        assert_eq!(a.data(&raw, 0).unwrap(), &[1u8; 16][..]);
        assert_eq!(a.data(&raw, 2).unwrap().len(), 0x801);
        assert_eq!(a.tail, 0);
        assert!(a.ids_follow_sectors());
    }

    #[test]
    fn the_id_rule_notices_a_wrong_id() {
        let raw = archive(&[16, 16], &[9, 0]);
        assert!(!parse(&raw).unwrap().ids_follow_sectors());
    }

    #[test]
    fn a_cut_or_foreign_file_is_an_error() {
        assert!(parse(b"XX\0\0\0\0\0\0").is_err());
        assert!(parse(b"LF").is_err());
        let raw = archive(&[0x1000], &[0]);
        assert!(parse(&raw[..raw.len() - 0x900]).is_err());
    }
}
