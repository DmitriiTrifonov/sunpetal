//! ISO 9660 of the PS2 disc: list and read files by path (У1.1). Only what the disc uses: 2048-byte
//! sectors, the primary volume descriptor, directory records; no Joliet, no Rock Ridge.
use std::io::{Read, Seek, SeekFrom};

pub const SECTOR: u64 = 2048;

/// A directory record: where a file or a directory lies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Name without the `;1` version suffix.
    pub name: String,
    pub lba: u32,
    pub size: u32,
    pub dir: bool,
}

pub struct Iso<R> {
    r: R,
    root: Entry,
}

fn record(b: &[u8]) -> Option<Entry> {
    let n = *b.first()? as usize;
    if n < 34 || b.len() < n {
        return None;
    }
    let name_len = b[32] as usize;
    if 33 + name_len > n {
        return None;
    }
    let name = b[33..33 + name_len].iter().map(|&c| c as char).collect::<String>();
    Some(Entry {
        name: name.split(';').next().unwrap_or("").to_string(),
        lba: u32::from_le_bytes(b[2..6].try_into().ok()?),
        size: u32::from_le_bytes(b[10..14].try_into().ok()?),
        dir: b[25] & 2 != 0,
    })
}

impl<R: Read + Seek> Iso<R> {
    pub fn open(mut r: R) -> Result<Self, String> {
        let mut pvd = vec![0u8; SECTOR as usize];
        r.seek(SeekFrom::Start(16 * SECTOR)).map_err(|e| e.to_string())?;
        r.read_exact(&mut pvd).map_err(|e| format!("volume descriptor: {e}"))?;
        if &pvd[1..6] != b"CD001" {
            return Err("not an ISO 9660 image".into());
        }
        let root = record(&pvd[156..190]).ok_or("root directory record")?;
        Ok(Iso { r, root })
    }

    pub fn read(&mut self, lba: u32, size: u32) -> Result<Vec<u8>, String> {
        let mut v = vec![0u8; size as usize];
        self.r.seek(SeekFrom::Start(lba as u64 * SECTOR)).map_err(|e| e.to_string())?;
        self.r.read_exact(&mut v).map_err(|e| format!("sector {lba}: {e}"))?;
        Ok(v)
    }

    /// The entries of a directory (without `.` and `..`).
    pub fn entries(&mut self, dir: &Entry) -> Result<Vec<Entry>, String> {
        let data = self.read(dir.lba, dir.size)?;
        let (mut out, mut i) = (Vec::new(), 0usize);
        while i < data.len() {
            let n = data[i] as usize;
            if n == 0 {
                i = (i / SECTOR as usize + 1) * SECTOR as usize; // the rest of the sector is padding
                continue;
            }
            let r = data.get(i..i + n).and_then(record).ok_or("broken directory record")?;
            i += n;
            if r.name != "\0" && r.name != "\u{1}" {
                out.push(r);
            }
        }
        Ok(out)
    }

    pub fn root(&self) -> &Entry {
        &self.root
    }

    /// The entry at `path` (`DATA/NAKA/X.BIN`, case-insensitive, `/` separated).
    pub fn find(&mut self, path: &str) -> Result<Entry, String> {
        let mut d = self.root.clone();
        for part in path.split('/').filter(|p| !p.is_empty()) {
            let list = self.entries(&d)?;
            d = list.into_iter().find(|e| e.name.eq_ignore_ascii_case(part)).ok_or_else(|| format!("no such path: {path}"))?;
        }
        Ok(d)
    }

    /// The bytes of the file at `path`.
    pub fn file(&mut self, path: &str) -> Result<Vec<u8>, String> {
        let e = self.find(path)?;
        if e.dir {
            return Err(format!("{path} is a directory"));
        }
        self.read(e.lba, e.size)
    }

    /// Every file under `dir` as (path, entry), depth first, in directory order.
    pub fn walk(&mut self, dir: &Entry, prefix: &str, out: &mut Vec<(String, Entry)>) -> Result<(), String> {
        for e in self.entries(dir)? {
            let path = format!("{prefix}{}", e.name);
            if e.dir {
                self.walk(&e, &format!("{path}/"), out)?;
            } else {
                out.push((path, e));
            }
        }
        Ok(())
    }

    /// All files of the disc.
    pub fn files(&mut self) -> Result<Vec<(String, Entry)>, String> {
        let mut out = Vec::new();
        let root = self.root.clone();
        self.walk(&root, "", &mut out)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn rec(name: &str, lba: u32, size: u32, dir: bool) -> Vec<u8> {
        let n = 33 + name.len() + (name.len() + 1) % 2;
        let mut r = vec![0u8; n];
        r[0] = n as u8;
        r[2..6].copy_from_slice(&lba.to_le_bytes());
        r[10..14].copy_from_slice(&size.to_le_bytes());
        r[25] = if dir { 2 } else { 0 };
        r[32] = name.len() as u8;
        r[33..33 + name.len()].copy_from_slice(name.as_bytes());
        r
    }

    /// A disc: root (sector 18) holds `A.TXT;1` (sector 20) and `SUB` (sector 19) with `B.BIN;1` (sector 21).
    fn disc() -> Vec<u8> {
        let mut d = vec![0u8; 22 * 2048];
        d[16 * 2048 + 1..16 * 2048 + 6].copy_from_slice(b"CD001");
        d[16 * 2048 + 156..16 * 2048 + 156 + 34].copy_from_slice(&rec("\0", 18, 2048, true)[..34]);
        let put = |d: &mut Vec<u8>, at: usize, recs: &[Vec<u8>]| {
            let mut o = at;
            for r in recs {
                d[o..o + r.len()].copy_from_slice(r);
                o += r.len();
            }
        };
        put(&mut d, 18 * 2048, &[rec("\0", 18, 2048, true), rec("\u{1}", 18, 2048, true), rec("A.TXT;1", 20, 5, false), rec("SUB", 19, 2048, true)]);
        put(&mut d, 19 * 2048, &[rec("\0", 19, 2048, true), rec("\u{1}", 18, 2048, true), rec("B.BIN;1", 21, 3, false)]);
        d[20 * 2048..20 * 2048 + 5].copy_from_slice(b"hello");
        d[21 * 2048..21 * 2048 + 3].copy_from_slice(&[1, 2, 3]);
        d
    }

    #[test]
    fn it_lists_and_reads_files_by_path() {
        let mut iso = Iso::open(Cursor::new(disc())).unwrap();
        assert_eq!(iso.file("a.txt").unwrap(), b"hello");
        assert_eq!(iso.file("SUB/B.BIN").unwrap(), [1, 2, 3]);
        let files = iso.files().unwrap();
        assert_eq!(files.iter().map(|(p, e)| (p.as_str(), e.size)).collect::<Vec<_>>(), [("A.TXT", 5), ("SUB/B.BIN", 3)]);
        assert!(iso.file("SUB").is_err());
        assert!(iso.find("NOPE").is_err());
    }

    #[test]
    fn it_rejects_what_is_not_a_disc() {
        assert!(Iso::open(Cursor::new(vec![0u8; 20 * 2048])).is_err());
        assert!(Iso::open(Cursor::new(vec![0u8; 100])).is_err());
    }
}
