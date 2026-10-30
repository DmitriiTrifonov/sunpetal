//! A cheap name for a big file by what is in it: for a cache that must not start over because a disc
//! image was copied or renamed (its modification time changes), and must not be taken for another
//! disc (the size and three samples differ).
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::delta::crc32;

const SAMPLE: u64 = 1 << 20;

/// `size-crc` of three MiB of the file: its start, its middle and its end. A file shorter than
/// three samples is read whole.
pub fn fingerprint(path: &Path) -> Result<String, String> {
    let bad = |e: std::io::Error| format!("{}: {}", path.display(), e);
    let mut f = File::open(path).map_err(bad)?;
    let size = f.metadata().map_err(bad)?.len();
    let mut all = Vec::new();
    let at = if size <= 3 * SAMPLE { vec![0] } else { vec![0, (size - SAMPLE) / 2, size - SAMPLE] };
    for from in at {
        f.seek(SeekFrom::Start(from)).map_err(bad)?;
        f.by_ref().take(SAMPLE).read_to_end(&mut all).map_err(bad)?;
    }
    Ok(format!("{}-{:08X}", size, crc32(&all)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_ignores_the_time_and_the_name_and_tells_files_apart() {
        let dir = std::env::temp_dir().join(format!("sunpetal-fp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut big: Vec<u8> = (0..5 << 20).map(|i: u32| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
        std::fs::write(dir.join("a.iso"), &big).unwrap();
        let a = fingerprint(&dir.join("a.iso")).unwrap();
        // a copy under another name, written later
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::copy(dir.join("a.iso"), dir.join("b.iso")).unwrap();
        assert_eq!(fingerprint(&dir.join("b.iso")).unwrap(), a);
        // a byte changed in the middle sample, one in the last, and a file one byte longer
        let mid = big.len() / 2;
        big[mid] ^= 1;
        std::fs::write(dir.join("c.iso"), &big).unwrap();
        assert_ne!(fingerprint(&dir.join("c.iso")).unwrap(), a);
        big[mid] ^= 1;
        let last = big.len() - 1;
        big[last] ^= 1;
        std::fs::write(dir.join("d.iso"), &big).unwrap();
        assert_ne!(fingerprint(&dir.join("d.iso")).unwrap(), a);
        big.push(0);
        std::fs::write(dir.join("e.iso"), &big).unwrap();
        assert_ne!(fingerprint(&dir.join("e.iso")).unwrap(), a);
        // a small file is read whole
        std::fs::write(dir.join("s"), b"small").unwrap();
        assert!(fingerprint(&dir.join("s")).unwrap().starts_with("5-"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
