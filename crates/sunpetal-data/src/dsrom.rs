//! The `NDS ` section: the DS ROM image inside the data file, so that the game starts from
//! `sunpetal.data` alone. The cartridge's trailing 0xFF padding (most of a 64 MB image) is not
//! stored.
//!
//! ```text
//! "NDS1", u32 length of the whole image, then the image up to its last byte that is not 0xFF
//! ```
use crate::container::Container;
use std::path::Path;

pub const TAG: &[u8; 4] = b"NDS ";

/// The section's bytes for a ROM image.
pub fn pack(rom: &[u8]) -> Result<Vec<u8>, String> {
    if rom.len() < 0x200 || u32::try_from(rom.len()).is_err() {
        return Err("this is not a DS ROM image".into());
    }
    let used = rom.iter().rposition(|b| *b != 0xFF).map_or(0, |i| i + 1).max(0x200);
    let mut out = Vec::with_capacity(8 + used);
    out.extend_from_slice(b"NDS1");
    out.extend_from_slice(&(rom.len() as u32).to_le_bytes());
    out.extend_from_slice(&rom[..used]);
    Ok(out)
}

/// The ROM image from the section's bytes.
pub fn unpack(section: &[u8]) -> Result<Vec<u8>, String> {
    if section.get(..4) != Some(b"NDS1".as_slice()) || section.len() < 8 + 0x200 {
        return Err("the ROM section is not understood (a newer file?)".into());
    }
    let len = u32::from_le_bytes(section[4..8].try_into().unwrap()) as usize;
    let body = &section[8..];
    if body.len() > len {
        return Err("the ROM section is longer than the image it holds".into());
    }
    let mut rom = body.to_vec();
    rom.resize(len, 0xFF);
    Ok(rom)
}

/// The ROM image kept in a data file, if it keeps one.
pub fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let c = Container::read_only(path, |t| t == TAG)?;
    c.get(TAG).map(|s| unpack(&s.data)).transpose()
}

/// Whether the data file keeps a ROM image (only its section table is read).
pub fn has_rom(path: &Path) -> bool {
    Container::tags(path).map_or(false, |t| t.contains(TAG))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_padding_comes_back() {
        let mut rom = vec![7u8; 0x300];
        rom.resize(0x1000, 0xFF);
        let s = pack(&rom).unwrap();
        assert_eq!(s.len(), 8 + 0x300);
        assert_eq!(unpack(&s).unwrap(), rom);
        // an image that ends in data keeps all of it
        let full = vec![1u8; 0x400];
        assert_eq!(unpack(&pack(&full).unwrap()).unwrap(), full);
    }
}
