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

    /// The file as bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let head = MAGIC.len() + 4 + 20 * self.sections.len();
        let mut out = Vec::with_capacity(head + self.sections.iter().map(|s| s.data.len()).sum::<usize>());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(self.sections.len() as u32).to_le_bytes());
        let mut at = head as u64;
        for s in &self.sections {
            out.extend_from_slice(&s.tag);
            out.extend_from_slice(&at.to_le_bytes());
            out.extend_from_slice(&(s.data.len() as u64).to_le_bytes());
            at += s.data.len() as u64;
        }
        let mut at = head as u64;
        for s in &self.sections {
            if &s.tag == b"TEXS" {
                out.extend_from_slice(&rebase_textures(&s.data, at as i64 - s.offset as i64)?);
            } else {
                out.extend_from_slice(&s.data);
            }
            at += s.data.len() as u64;
        }
        Ok(out)
    }

    /// Writes the file through a temporary one, so that it is whole or not there.
    pub fn write(&self, path: &Path) -> Result<(), String> {
        let bytes = self.to_bytes()?;
        let part = path.with_extension("part");
        fs::write(&part, bytes).map_err(|e| format!("{}: {}", part.display(), e))?;
        fs::rename(&part, path).map_err(|e| format!("{}: {}", path.display(), e))
    }
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
            c.add(b"META", b"xyz".to_vec());
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
}
