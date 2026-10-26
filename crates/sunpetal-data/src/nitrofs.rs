//! The file system of a DS cartridge (NitroFS): which file a byte of the ROM belongs to (the debug
//! tools name the files the game reads) and the overlay that replaces files of the image (the
//! translation: `apply_overlay`).
use std::collections::HashMap;

fn u32_at(rom: &[u8], at: usize) -> Result<u32, String> {
    rom.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or_else(|| format!("ROM ends at {:X}", at))
}

fn u16_at(rom: &[u8], at: usize) -> Result<u16, String> {
    rom.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or_else(|| format!("ROM ends at {:X}", at))
}

/// The files of a ROM, by place and by path.
pub struct NitroFs {
    /// (start, end, path), sorted by start.
    files: Vec<(u32, u32, String)>,
    by_path: HashMap<String, usize>,
    /// Where the FAT is, and the FAT index of each file, by path.
    fat: usize,
    ids: HashMap<String, usize>,
}

impl NitroFs {
    pub fn parse(rom: &[u8]) -> Result<NitroFs, String> {
        let (fnt, fat) = (u32_at(rom, 0x40)? as usize, u32_at(rom, 0x48)? as usize);
        let fat_len = u32_at(rom, 0x4C)? as usize;
        let mut places = Vec::new();
        for i in 0..fat_len / 8 {
            places.push((u32_at(rom, fat + 8 * i)?, u32_at(rom, fat + 8 * i + 4)?));
        }
        let mut files = Vec::new();
        let mut ids = HashMap::new();
        let mut dirs = vec![(0xF000u16, String::new())];
        while let Some((dir, prefix)) = dirs.pop() {
            let entry = fnt + 8 * (dir & 0xFFF) as usize;
            let mut at = fnt + u32_at(rom, entry)? as usize;
            let mut id = u16_at(rom, entry + 4)? as usize;
            loop {
                let tag = *rom.get(at).ok_or("the name table ends")?;
                at += 1;
                if tag == 0 {
                    break;
                }
                let len = (tag & 0x7F) as usize;
                let name = String::from_utf8_lossy(rom.get(at..at + len).ok_or("the name table ends")?).into_owned();
                at += len;
                let path = if prefix.is_empty() { name } else { format!("{}/{}", prefix, name) };
                if tag & 0x80 != 0 {
                    dirs.push((u16_at(rom, at)?, path));
                    at += 2;
                } else {
                    let (start, end) = *places.get(id).ok_or("a file without a place")?;
                    ids.insert(path.clone(), id);
                    files.push((start, end, path));
                    id += 1;
                }
            }
        }
        files.sort();
        let by_path = files.iter().enumerate().map(|(i, f)| (f.2.clone(), i)).collect();
        Ok(NitroFs { files, by_path, fat, ids })
    }

    /// The file that the ROM's byte `offset` belongs to.
    pub fn at(&self, offset: u32) -> Option<&str> {
        let i = self.files.partition_point(|f| f.0 <= offset).checked_sub(1)?;
        let f = &self.files[i];
        (offset < f.1).then_some(f.2.as_str())
    }

    /// The paths of all the files.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(|f| f.2.as_str())
    }

    /// The bytes of the file `path` in `rom`.
    pub fn file<'a>(&self, rom: &'a [u8], path: &str) -> Option<&'a [u8]> {
        let (start, end) = self.place(path)?;
        rom.get(start as usize..end as usize)
    }

    /// Where the file `path` is in the ROM: (start, end).
    pub fn place(&self, path: &str) -> Option<(u32, u32)> {
        self.by_path.get(path).map(|i| (self.files[*i].0, self.files[*i].1))
    }
}

/// Replaces files of the image in memory: a file that fits its place is written there (the FAT
/// keeps its start, its end moves), a larger one is appended to the end of the image (the FAT
/// points to it). The other files stay where they are; the header is not touched. Returns how
/// many files were replaced.
pub fn apply_overlay(rom: &mut Vec<u8>, files: &[(String, Vec<u8>)]) -> Result<usize, String> {
    let fs = NitroFs::parse(rom)?;
    for (path, data) in files {
        let id = *fs.ids.get(path).ok_or_else(|| format!("the overlay has {}, the ROM does not", path))?;
        let (start, end) = fs.place(path).ok_or("a file without a place")?;
        let entry = fs.fat + 8 * id;
        let start = if data.len() <= (end - start) as usize {
            start as usize
        } else {
            while rom.len() % 4 != 0 {
                rom.push(0xFF);
            }
            let at = rom.len();
            rom.resize(at + data.len(), 0xFF);
            at
        };
        rom[start..start + data.len()].copy_from_slice(data);
        rom[entry..entry + 4].copy_from_slice(&(start as u32).to_le_bytes());
        rom[entry + 4..entry + 8].copy_from_slice(&((start + data.len()) as u32).to_le_bytes());
    }
    Ok(files.len())
}

/// The files of a folder laid out as the NitroFS (`Chapter/Pack/C00/ue/C00_01.txt.z`), by path.
pub fn read_overlay_dir(dir: &std::path::Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    fn walk(dir: &std::path::Path, prefix: &str, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
        let mut entries: Vec<_> = std::fs::read_dir(dir).map_err(|e| format!("{}: {}", dir.display(), e))?.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = if prefix.is_empty() { name } else { format!("{}/{}", prefix, name) };
            if e.path().is_dir() {
                walk(&e.path(), &path, out)?;
            } else {
                out.push((path, std::fs::read(e.path()).map_err(|er| format!("{}: {}", e.path().display(), er))?));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, "", &mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ROM with two files, `a.bin` (4 bytes) and `d/b.bin` (8 bytes).
    fn tiny_rom() -> Vec<u8> {
        let mut r = vec![0u8; 0x400];
        let (fnt, fat) = (0x200usize, 0x300usize);
        r[0x40..0x44].copy_from_slice(&(fnt as u32).to_le_bytes());
        r[0x48..0x4C].copy_from_slice(&(fat as u32).to_le_bytes());
        r[0x4C..0x50].copy_from_slice(&16u32.to_le_bytes());
        // root: subtable at +16, first file 0, two directories counted
        r[fnt..fnt + 4].copy_from_slice(&16u32.to_le_bytes());
        r[fnt + 4..fnt + 6].copy_from_slice(&0u16.to_le_bytes());
        r[fnt + 6..fnt + 8].copy_from_slice(&2u16.to_le_bytes());
        // dir F001: subtable at +40, first file 1, parent F000
        r[fnt + 8..fnt + 12].copy_from_slice(&40u32.to_le_bytes());
        r[fnt + 12..fnt + 14].copy_from_slice(&1u16.to_le_bytes());
        r[fnt + 14..fnt + 16].copy_from_slice(&0xF000u16.to_le_bytes());
        let mut sub = vec![5u8];
        sub.extend(b"a.bin");
        sub.push(0x81);
        sub.push(b'd');
        sub.extend(&0xF001u16.to_le_bytes());
        sub.push(0);
        r[fnt + 16..fnt + 16 + sub.len()].copy_from_slice(&sub);
        let mut sub = vec![5u8];
        sub.extend(b"b.bin");
        sub.push(0);
        r[fnt + 40..fnt + 40 + sub.len()].copy_from_slice(&sub);
        for (i, (s, e)) in [(0x100u32, 0x104u32), (0x110, 0x118)].iter().enumerate() {
            r[fat + 8 * i..fat + 8 * i + 4].copy_from_slice(&s.to_le_bytes());
            r[fat + 8 * i + 4..fat + 8 * i + 8].copy_from_slice(&e.to_le_bytes());
        }
        r[0x100..0x104].copy_from_slice(b"AAAA");
        r[0x110..0x118].copy_from_slice(b"BBBBBBBB");
        r
    }

    #[test]
    fn a_smaller_file_goes_to_its_place_a_larger_one_to_the_end() {
        let mut rom = tiny_rom();
        let before = rom.len();
        let n = apply_overlay(&mut rom, &[("a.bin".into(), b"xy".to_vec()), ("d/b.bin".into(), b"0123456789AB".to_vec())]).unwrap();
        assert_eq!(n, 2);
        let fs = NitroFs::parse(&rom).unwrap();
        assert_eq!(fs.place("a.bin"), Some((0x100, 0x102)));
        assert_eq!(&rom[0x100..0x102], b"xy");
        let (s, e) = fs.place("d/b.bin").unwrap();
        assert_eq!((s as usize, e as usize), (before, before + 12));
        assert_eq!(&rom[s as usize..e as usize], b"0123456789AB");
        assert_eq!(fs.at(before as u32 + 3), Some("d/b.bin"));
    }

    #[test]
    fn a_file_the_rom_lacks_is_an_error() {
        let mut rom = tiny_rom();
        let err = apply_overlay(&mut rom, &[("nope.bin".into(), vec![1])]).unwrap_err();
        assert!(err.contains("nope.bin"));
    }
}
