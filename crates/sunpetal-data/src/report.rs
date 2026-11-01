//! What a data file holds, for the player and the builder: the check of `META` and a readable list
//! of the parts (`sunpetal-check`).
use std::path::Path;

use crate::container::{verify_file, Container};
use crate::translate::Translation;

fn size(n: u64) -> String {
    if n >= 1 << 20 {
        format!("{:.1} MB", n as f64 / (1 << 20) as f64)
    } else {
        format!("{:.1} KB", n as f64 / 1024.0)
    }
}

/// Checks the file as the game would (`verify_file`) and says what is inside. A file without
/// `META` is listed but said to be unchecked; a damaged or cut file is an error.
pub fn report(path: &Path) -> Result<String, String> {
    let meta = verify_file(path)?;
    let c = Container::read_only(path, |t| t == b"TRAN")?;
    let mut o = format!("{}\n", path.display());
    match &meta {
        Some(m) => {
            o += &format!("game {}, ROM CRC-32 {:08X}, built by {}\n", String::from_utf8_lossy(&m.game), m.rom_crc, if m.builder.is_empty() { "?" } else { &m.builder });
            for p in &m.parts {
                let what = match &p.tag {
                    b"NDS " => "the DS ROM",
                    b"TEXS" => "PS2 textures",
                    b"TRAN" => "a translation",
                    b"2DPK" => "2D pictures",
                    _ => "(a part this program does not know)",
                };
                o += &format!("  {}  {:>9}  CRC-32 {:08X}  {}\n", String::from_utf8_lossy(&p.tag), size(p.len), p.crc, what);
            }
            o += &format!("{} parts, every length and checksum holds\n", m.parts.len());
        }
        None => o += "no META: the file was made before it and is not checked\n",
    }
    for s in c.all(b"TRAN") {
        match Translation::from_bytes(&s.data) {
            Ok(t) => o += &format!("translation `{}`: {} texts, for ROM CRC-32 {:08X}\n", t.lang, t.table.texts(), t.rom_crc),
            Err(e) => o += &format!("a translation that cannot be read: {}\n", e),
        }
    }
    Ok(o)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn it_lists_the_parts_and_refuses_a_damaged_file() {
        let mut c = Container::new();
        c.add(b"TEXS", vec![0, 0, 0, 0]);
        c.add(b"TRAN", b"not a translation".to_vec());
        c.set_rom(*b"YHNP", 0x1234_ABCD);
        let dir = std::env::temp_dir().join(format!("sunpetal-report-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("r.spdata");
        c.write(&path).unwrap();
        let r = report(&path).unwrap();
        assert!(r.contains("YHNP") && r.contains("1234ABCD") && r.contains("PS2 textures") && r.contains("cannot be read"), "{}", r);
        let mut bytes = fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&path, bytes).unwrap();
        let e = report(&path).unwrap_err();
        assert!(e.contains("damaged"), "{}", e);
        fs::remove_dir_all(&dir).unwrap();
    }
}
