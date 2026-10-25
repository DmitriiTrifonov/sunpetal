//! The file system of a DS cartridge (NitroFS): which file a byte of the ROM belongs to (feature
//! `debug-tools`: the tools name the files the game reads).
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
                    files.push((start, end, path));
                    id += 1;
                }
            }
        }
        files.sort();
        let by_path = files.iter().enumerate().map(|(i, f)| (f.2.clone(), i)).collect();
        Ok(NitroFs { files, by_path })
    }

    /// The file that the ROM's byte `offset` belongs to.
    pub fn at(&self, offset: u32) -> Option<&str> {
        let i = self.files.partition_point(|f| f.0 <= offset).checked_sub(1)?;
        let f = &self.files[i];
        (offset < f.1).then_some(f.2.as_str())
    }

    /// Where the file `path` is in the ROM: (start, end).
    pub fn place(&self, path: &str) -> Option<(u32, u32)> {
        self.by_path.get(path).map(|i| (self.files[*i].0, self.files[*i].1))
    }
}
