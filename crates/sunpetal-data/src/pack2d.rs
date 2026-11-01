//! The 2D pictures part of a data file (`P2D1`, stage 8е.5, `docs/2D-UPSCALE.md`): the folder of PNGs a 2D pack
//! is (named as the runtime's `dump_2d` names them) in one section.
//!
//! ```text
//! "P2D1", u32 count, per picture: u16 name length, name, u32 length, the PNG
//! ```
use std::path::Path;

/// The section's tag in a data file.
pub const TAG: &[u8; 4] = b"2DPK";
const MAGIC: &[u8; 4] = b"P2D1";

/// Whether `name` is a picture of a 2D pack (a sprite `obj…` or a background layer `bg…`, a PNG).
pub fn is_picture(name: &str) -> bool {
    name.ends_with(".png") && (name.starts_with("obj") || name.starts_with("bg"))
}

/// The pictures of a folder as a section, in the order of their names.
pub fn pack_dir(dir: &Path) -> Result<Vec<u8>, String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {}", dir.display(), e))?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| is_picture(n))
        .collect();
    names.sort();
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&(names.len() as u32).to_le_bytes());
    for n in &names {
        let data = std::fs::read(dir.join(n)).map_err(|e| format!("{}: {}", n, e))?;
        out.extend_from_slice(&(n.len() as u16).to_le_bytes());
        out.extend_from_slice(n.as_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
    }
    Ok(out)
}

/// The pictures of a section: (name, PNG bytes).
pub fn unpack(d: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let bad = || "2D pictures part: cut short or not one".to_string();
    if d.len() < 8 || &d[..4] != MAGIC {
        return Err(bad());
    }
    let n = u32::from_le_bytes(d[4..8].try_into().unwrap()) as usize;
    let mut at = 8;
    let mut out = Vec::new();
    for _ in 0..n {
        let nl = u16::from_le_bytes(d.get(at..at + 2).ok_or_else(bad)?.try_into().unwrap()) as usize;
        let name = String::from_utf8_lossy(d.get(at + 2..at + 2 + nl).ok_or_else(bad)?).into_owned();
        at += 2 + nl;
        let len = u32::from_le_bytes(d.get(at..at + 4).ok_or_else(bad)?.try_into().unwrap()) as usize;
        let data = d.get(at + 4..at + 4 + len).ok_or_else(bad)?.to_vec();
        at += 4 + len;
        out.push((name, data));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_goes_into_a_section_and_comes_back() {
        let dir = std::env::temp_dir().join(format!("sunpetal-p2d-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("obj0_8x8_00000000000000aa_00000000000000bb.png"), b"png one").unwrap();
        std::fs::write(dir.join("bg00_00000000000000cc.png"), b"png two").unwrap();
        std::fs::write(dir.join("index.csv"), b"left out").unwrap();
        let d = pack_dir(&dir).unwrap();
        let back = unpack(&d).unwrap();
        assert_eq!(back.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["bg00_00000000000000cc.png", "obj0_8x8_00000000000000aa_00000000000000bb.png"]);
        assert_eq!(back[1].1, b"png one");
        assert!(unpack(&d[..d.len() - 1]).is_err());
        assert!(unpack(b"nope").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
