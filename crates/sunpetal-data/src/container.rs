//! The Sunpetal data file (`sunpetal.data`, `.spdata`): the game's resources in sections of one
//! file, built on the player's machine from their own copies of the game and never shared
//! (docs/PLAN.md, stage 9b). The layout is that of `tools/ps2/spdata.py`, whose version 1 this
//! reads and writes:
//!
//! ```text
//! 0   8 bytes  "SPDATA\0" and the version byte (1)
//! 8   u32      the number of sections
//! 12  per section: 4-byte tag, u64 offset, u64 length (20 bytes each)
//! ```
//!
//! A reader skips the tags it does not know. Tags: `TEXS` (texture replacements, `texpack.rs`),
//! `TRAN` (a translation, `translate::Translation`; one section per language), `META`.
//! `TEXS` holds absolute offsets of its pictures, so `write` moves them with the section.
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::delta::{crc32, Crc32};

pub const MAGIC: &[u8; 8] = b"SPDATA\0\x01";
const MAX_SECTIONS: usize = 4096;

/// A section of a file: its tag, where it was in the file it was read from (for `TEXS`), and its bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub tag: [u8; 4],
    pub offset: u64,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Container {
    pub sections: Vec<Section>,
}

impl Container {
    pub fn new() -> Container {
        Container::default()
    }

    /// Reads all the sections of a file into memory.
    pub fn read(path: &Path) -> Result<Container, String> {
        Container::read_only(path, |_| true)
    }

    /// Reads the sections whose tag `want` takes (the others, a texture pack of hundreds of
    /// megabytes, stay in the file).
    pub fn read_only(path: &Path, want: impl Fn(&[u8; 4]) -> bool) -> Result<Container, String> {
        let bad = |why: &str| format!("data file {}: {}", path.display(), why);
        let mut f = fs::File::open(path).map_err(|e| bad(&e.to_string()))?;
        let mut head = [0u8; 12];
        f.read_exact(&mut head).map_err(|_| bad("too short"))?;
        if &head[..8] != MAGIC {
            return Err(bad("not a Sunpetal data file, or a newer version of it"));
        }
        let n = u32::from_le_bytes(head[8..12].try_into().unwrap()) as usize;
        if n > MAX_SECTIONS {
            return Err(bad("too many sections"));
        }
        let mut index = Vec::new();
        for _ in 0..n {
            let mut s = [0u8; 20];
            f.read_exact(&mut s).map_err(|_| bad("section table cut short"))?;
            index.push((<[u8; 4]>::try_from(&s[..4]).unwrap(), u64::from_le_bytes(s[4..12].try_into().unwrap()), u64::from_le_bytes(s[12..20].try_into().unwrap())));
        }
        let size = f.metadata().map_err(|e| bad(&e.to_string()))?.len();
        let mut sections = Vec::new();
        for (tag, offset, len) in index {
            if !want(&tag) {
                continue;
            }
            if offset.checked_add(len).map_or(true, |end| end > size) {
                return Err(bad("a section reaches past the end"));
            }
            f.seek(SeekFrom::Start(offset)).map_err(|e| bad(&e.to_string()))?;
            let mut data = vec![0; len as usize];
            f.read_exact(&mut data).map_err(|e| bad(&e.to_string()))?;
            sections.push(Section { tag, offset, data });
        }
        Ok(Container { sections })
    }

    /// The tags of a file's sections, in order, from its section table alone.
    pub fn tags(path: &Path) -> Result<Vec<[u8; 4]>, String> {
        let bad = |why: &str| format!("data file {}: {}", path.display(), why);
        let mut f = fs::File::open(path).map_err(|e| bad(&e.to_string()))?;
        let mut head = [0u8; 12];
        f.read_exact(&mut head).map_err(|_| bad("too short"))?;
        if &head[..8] != MAGIC {
            return Err(bad("not a Sunpetal data file, or a newer version of it"));
        }
        let n = u32::from_le_bytes(head[8..12].try_into().unwrap()) as usize;
        if n > MAX_SECTIONS {
            return Err(bad("too many sections"));
        }
        let mut tags = Vec::new();
        for _ in 0..n {
            let mut s = [0u8; 20];
            f.read_exact(&mut s).map_err(|_| bad("section table cut short"))?;
            tags.push(s[..4].try_into().unwrap());
        }
        Ok(tags)
    }

    /// The sections with `tag`.
    pub fn all(&self, tag: &[u8; 4]) -> impl Iterator<Item = &Section> {
        let tag = *tag;
        self.sections.iter().filter(move |s| s.tag == tag)
    }

    pub fn get(&self, tag: &[u8; 4]) -> Option<&Section> {
        self.all(tag).next()
    }

    /// Adds a section after the others.
    pub fn add(&mut self, tag: &[u8; 4], data: Vec<u8>) {
        self.sections.push(Section { tag: *tag, offset: 0, data });
    }

    /// Drops the sections with `tag`.
    pub fn remove(&mut self, tag: &[u8; 4]) {
        self.sections.retain(|s| &s.tag != tag);
    }

    /// The file as bytes. The `META` section, if there is one, gets the length and checksum of
    /// every other section as written.
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let mut head_at = (MAGIC.len() + 4 + 20 * self.sections.len()) as u64;
        let mut datas: Vec<Vec<u8>> = Vec::with_capacity(self.sections.len());
        // the offsets of TEXS depend on where it lands, and META's size does not depend on its parts' values
        let meta_at = self.sections.iter().position(|s| &s.tag == b"META");
        let meta_len = match meta_at {
            Some(i) => Meta::from_bytes(&self.sections[i].data)?.with_parts(self.sections.len() - 1).to_bytes().len(),
            None => 0,
        };
        let mut at = head_at;
        for s in &self.sections {
            let d = if &s.tag == b"TEXS" {
                rebase_textures(&s.data, at as i64 - s.offset as i64)?
            } else if &s.tag == b"META" {
                vec![0; meta_len]
            } else {
                s.data.clone()
            };
            at += d.len() as u64;
            datas.push(d);
        }
        if let Some(i) = meta_at {
            let mut m = Meta::from_bytes(&self.sections[i].data)?;
            m.parts = self.sections.iter().zip(&datas).enumerate().filter(|(j, _)| *j != i).map(|(_, (s, d))| Part { tag: s.tag, len: d.len() as u64, crc: crc32(d) }).collect();
            datas[i] = m.to_bytes();
        }
        let mut out = Vec::with_capacity(head_at as usize + datas.iter().map(|d| d.len()).sum::<usize>());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(self.sections.len() as u32).to_le_bytes());
        for (s, d) in self.sections.iter().zip(&datas) {
            out.extend_from_slice(&s.tag);
            out.extend_from_slice(&head_at.to_le_bytes());
            out.extend_from_slice(&(d.len() as u64).to_le_bytes());
            head_at += d.len() as u64;
        }
        for d in &datas {
            out.extend_from_slice(d);
        }
        Ok(out)
    }

    /// Notes which ROM the file is built from (`META`); the parts are filled in when it is written.
    pub fn set_rom(&mut self, game: [u8; 4], rom_crc: u32) {
        self.remove(b"META");
        let meta = Meta { game, rom_crc, builder: format!("sunpetal-data {}", env!("CARGO_PKG_VERSION")), parts: Vec::new() };
        self.sections.insert(0, Section { tag: *b"META", offset: 0, data: meta.to_bytes() });
    }

    /// The file's `META`, if it has one.
    pub fn meta(&self) -> Result<Option<Meta>, String> {
        self.get(b"META").map(|s| Meta::from_bytes(&s.data)).transpose()
    }

    /// Writes the file through a temporary one, so that it is whole or not there.
    pub fn write(&self, path: &Path) -> Result<(), String> {
        let bytes = self.to_bytes()?;
        let part = path.with_extension("part");
        fs::write(&part, bytes).map_err(|e| format!("{}: {}", part.display(), e))?;
        fs::rename(&part, path).map_err(|e| format!("{}: {}", path.display(), e))
    }
}

/// What a section is, as `META` records it.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub tag: [u8; 4],
    pub len: u64,
    pub crc: u32,
}

/// `META`: which game the file is for and what its other sections must be.
///
/// ```text
/// u32 version (1), 4 bytes game code, u32 CRC-32 of the ROM, u32 parts,
/// per part: 4-byte tag, u64 length, u32 CRC-32; then u16 length and text of the builder
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Meta {
    pub game: [u8; 4],
    pub rom_crc: u32,
    pub builder: String,
    /// The other sections in the file's order.
    pub parts: Vec<Part>,
}

impl Meta {
    fn with_parts(mut self, n: usize) -> Meta {
        self.parts = vec![Part { tag: *b"    ", len: 0, crc: 0 }; n];
        self
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::new();
        o.extend_from_slice(&1u32.to_le_bytes());
        o.extend_from_slice(&self.game);
        o.extend_from_slice(&self.rom_crc.to_le_bytes());
        o.extend_from_slice(&(self.parts.len() as u32).to_le_bytes());
        for p in &self.parts {
            o.extend_from_slice(&p.tag);
            o.extend_from_slice(&p.len.to_le_bytes());
            o.extend_from_slice(&p.crc.to_le_bytes());
        }
        let b = self.builder.as_bytes();
        let b = &b[..b.len().min(u16::MAX as usize)];
        o.extend_from_slice(&(b.len() as u16).to_le_bytes());
        o.extend_from_slice(b);
        o
    }

    pub fn from_bytes(d: &[u8]) -> Result<Meta, String> {
        let cut = || "the META section is cut short".to_string();
        let take = |at: usize, n: usize| d.get(at..at + n).ok_or_else(cut);
        let version = u32::from_le_bytes(take(0, 4)?.try_into().unwrap());
        if version != 1 {
            return Err(format!("META version {}: this program reads 1, update it", version));
        }
        let game = take(4, 4)?.try_into().unwrap();
        let rom_crc = u32::from_le_bytes(take(8, 4)?.try_into().unwrap());
        let n = u32::from_le_bytes(take(12, 4)?.try_into().unwrap()) as usize;
        if n > MAX_SECTIONS {
            return Err("META lists too many parts".into());
        }
        let mut parts = Vec::with_capacity(n);
        let mut at = 16;
        for _ in 0..n {
            let b = take(at, 16)?;
            parts.push(Part { tag: b[..4].try_into().unwrap(), len: u64::from_le_bytes(b[4..12].try_into().unwrap()), crc: u32::from_le_bytes(b[12..16].try_into().unwrap()) });
            at += 16;
        }
        let bl = u16::from_le_bytes(take(at, 2)?.try_into().unwrap()) as usize;
        let builder = String::from_utf8_lossy(take(at + 2, bl)?).into_owned();
        Ok(Meta { game, rom_crc, builder, parts })
    }

    /// Whether a file built from another ROM is being played with this one: the message if so.
    pub fn check_rom(&self, rom_crc: u32) -> Result<(), String> {
        if self.rom_crc == rom_crc {
            Ok(())
        } else {
            Err(format!("the data file is built from another ROM (CRC-32 {:08X}; this one is {:08X})", self.rom_crc, rom_crc))
        }
    }

    /// Whether `rom` (a whole DS image) is the game the file was built for: the game code in its
    /// header, then its CRC-32, against `META`; the message for the player if not.
    pub fn check_image(&self, rom: &[u8]) -> Result<(), String> {
        let code = rom.get(0xC..0x10).ok_or("the ROM is too short to be a DS image")?;
        if code != self.game {
            return Err(format!("the data file is for the game {}, this ROM is {}", String::from_utf8_lossy(&self.game), String::from_utf8_lossy(code)));
        }
        self.check_rom(crate::delta::crc32(rom))
    }
}

fn name(tag: &[u8; 4]) -> String {
    String::from_utf8_lossy(tag).into_owned()
}

/// Checks a file against its `META` without keeping its sections in memory: the version, that
/// every section is there as long as recorded, and that its checksum holds. A file without
/// `META` (made before it) gives `None` and is not checked.
pub fn verify_file(path: &Path) -> Result<Option<Meta>, String> {
    let bad = |why: String| format!("data file {}: {}", path.display(), why);
    let mut f = fs::File::open(path).map_err(|e| bad(e.to_string()))?;
    let mut head = [0u8; 12];
    f.read_exact(&mut head).map_err(|_| bad("too short".into()))?;
    if &head[..8] != MAGIC {
        return Err(bad("not a Sunpetal data file, or a newer version of it".into()));
    }
    let n = u32::from_le_bytes(head[8..12].try_into().unwrap()) as usize;
    if n > MAX_SECTIONS {
        return Err(bad("too many sections".into()));
    }
    let mut index = Vec::new();
    for _ in 0..n {
        let mut s = [0u8; 20];
        f.read_exact(&mut s).map_err(|_| bad("section table cut short".into()))?;
        index.push((<[u8; 4]>::try_from(&s[..4]).unwrap(), u64::from_le_bytes(s[4..12].try_into().unwrap()), u64::from_le_bytes(s[12..20].try_into().unwrap())));
    }
    let size = f.metadata().map_err(|e| bad(e.to_string()))?.len();
    if index.iter().any(|(_, o, l)| o.checked_add(*l).map_or(true, |e| e > size)) {
        return Err(bad("a section reaches past the end (the file is cut short)".into()));
    }
    let Some(&(_, mo, ml)) = index.iter().find(|(t, _, _)| t == b"META") else {
        return Ok(None);
    };
    f.seek(SeekFrom::Start(mo)).map_err(|e| bad(e.to_string()))?;
    let mut raw = vec![0; ml as usize];
    f.read_exact(&mut raw).map_err(|e| bad(e.to_string()))?;
    let meta = Meta::from_bytes(&raw).map_err(bad)?;
    let others: Vec<_> = index.iter().filter(|(t, _, _)| t != b"META").collect();
    if others.len() != meta.parts.len() {
        return Err(bad(format!("{} sections, META lists {}", others.len(), meta.parts.len())));
    }
    let mut buf = vec![0u8; 1 << 20];
    for ((tag, off, len), part) in others.into_iter().zip(&meta.parts) {
        if *tag != part.tag || *len != part.len {
            return Err(bad(format!("section {} does not match META", name(tag))));
        }
        f.seek(SeekFrom::Start(*off)).map_err(|e| bad(e.to_string()))?;
        let (mut crc, mut left) = (Crc32::new(), *len);
        while left > 0 {
            let k = left.min(buf.len() as u64) as usize;
            f.read_exact(&mut buf[..k]).map_err(|e| bad(e.to_string()))?;
            crc.update(&buf[..k]);
            left -= k as u64;
        }
        if crc.finish() != part.crc {
            return Err(bad(format!("section {} is damaged (checksum differs); build the file again", name(tag))));
        }
    }
    Ok(Some(meta))
}

/// `TEXS`: u32 count, then 33 bytes per entry, the picture's absolute offset at 21..29.
fn rebase_textures(data: &[u8], by: i64) -> Result<Vec<u8>, String> {
    let mut out = data.to_vec();
    if by == 0 {
        return Ok(out);
    }
    let n = u32::from_le_bytes(data.get(..4).ok_or("the texture section is cut short")?.try_into().unwrap()) as usize;
    for i in 0..n {
        let at = 4 + 33 * i + 21;
        let old = u64::from_le_bytes(data.get(at..at + 8).ok_or("the texture index is cut short")?.try_into().unwrap());
        let new = u64::try_from(old as i64 + by).map_err(|_| "a picture moved before the start")?;
        out[at..at + 8].copy_from_slice(&new.to_le_bytes());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ROM image is the file's game when the game code and the CRC-32 both match `META`;
    /// another game code and another image are each told apart, in the player's words.
    #[test]
    fn a_rom_image_is_checked_against_meta_by_game_code_and_crc() {
        let mut rom = vec![0u8; 0x400];
        rom[0xC..0x10].copy_from_slice(b"YHNP");
        let meta = Meta { game: *b"YHNP", rom_crc: crate::delta::crc32(&rom), builder: String::new(), parts: Vec::new() };
        assert!(meta.check_image(&rom).is_ok());
        let mut other = rom.clone();
        other[0x100] = 1;
        let e = meta.check_image(&other).unwrap_err();
        assert!(e.contains("another ROM"), "{}", e);
        let mut jp = rom.clone();
        jp[0xC..0x10].copy_from_slice(b"YHNJ");
        let e = meta.check_image(&jp).unwrap_err();
        assert!(e.contains("YHNP") && e.contains("YHNJ"), "{}", e);
        assert!(meta.check_image(&rom[..8]).unwrap_err().contains("too short"));
    }

    #[test]
    fn it_writes_and_reads_sections_and_moves_the_texture_offsets() {
        // a TEXS with one entry whose picture is at 100 in a file whose head was 32 + 20 bytes
        let mut texs = vec![1, 0, 0, 0];
        let mut e = [0u8; 33];
        e[21..29].copy_from_slice(&100u64.to_le_bytes());
        texs.extend_from_slice(&e);
        let mut c = Container::new();
        c.sections.push(Section { tag: *b"TEXS", offset: 32, data: texs });
        c.add(b"TRAN", b"abc".to_vec());
        let dir = std::env::temp_dir().join(format!("sunpetal-container-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.spdata");
        c.write(&path).unwrap();
        let back = Container::read(&path).unwrap();
        assert_eq!(back.get(b"TRAN").unwrap().data, b"abc");
        let t = back.get(b"TEXS").unwrap();
        // the head is 12 + 2 * 20 bytes now; the section moved from 32 to 52
        assert_eq!(t.offset, 52);
        assert_eq!(u64::from_le_bytes(t.data[25..33].try_into().unwrap()), 120);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_cut_or_foreign_file_is_an_error() {
        let c = {
            let mut c = Container::new();
            c.add(b"JUNK", b"xyz".to_vec());
            c.to_bytes().unwrap()
        };
        let dir = std::env::temp_dir().join(format!("sunpetal-container2-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("cut"), &c[..c.len() - 1]).unwrap();
        fs::write(dir.join("foreign"), b"PK\x03\x04 and the rest of a zip").unwrap();
        assert!(Container::read(&dir.join("cut")).is_err());
        assert!(Container::read(&dir.join("foreign")).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn meta_records_the_rom_and_catches_a_damaged_part() {
        let mut c = Container::new();
        c.add(b"TRAN", b"some translation".to_vec());
        c.add(b"TRAN", b"another".to_vec());
        c.set_rom(*b"YHNP", 0xDEAD_BEEF);
        let dir = std::env::temp_dir().join(format!("sunpetal-container3-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("m.spdata");
        c.write(&path).unwrap();
        let m = verify_file(&path).unwrap().unwrap();
        assert_eq!((m.game, m.rom_crc, m.parts.len()), (*b"YHNP", 0xDEAD_BEEF, 2));
        assert!(m.check_rom(0xDEAD_BEEF).is_ok() && m.check_rom(1).is_err());
        // reading again and writing again gives the same file
        let back = Container::read(&path).unwrap();
        assert_eq!(back.to_bytes().unwrap(), fs::read(&path).unwrap());
        // one flipped byte in a part
        let mut bytes = fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&path, bytes).unwrap();
        let e = verify_file(&path).unwrap_err();
        assert!(e.contains("TRAN") && e.contains("damaged"), "{}", e);
        // a file without META is not checked
        let mut plain = Container::new();
        plain.add(b"TRAN", b"x".to_vec());
        plain.write(&path).unwrap();
        assert_eq!(verify_file(&path).unwrap(), None);
        fs::remove_dir_all(&dir).unwrap();
    }
}
