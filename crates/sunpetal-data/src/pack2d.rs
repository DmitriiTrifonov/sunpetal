//! The 2D pictures part of a data file (`P2D1`, stage 8е.5, `docs/2D-UPSCALE.md`): the folder of PNGs a 2D pack
//! is (named as the runtime's `dump_2d` names them) in one section.
//!
//! ```text
//! "P2D1", u32 count, per picture: u16 name length, name, u32 length, the PNG
//! ```
//!
//! A folder may have a manifest `pack.toml` that names pictures by id: `{id}.png` serves every key listed under
//! `[id]` (`keys = [...]`, or a list per language, `en = [...]`, `ru = [...]`; a key is a dump name without `.png`,
//! maybe with `@group.variant`). One picture then serves several keys (palettes, languages); the keys' own
//! files keep working without a manifest, and a file named by key wins over the manifest's.
use std::path::{Path, PathBuf};

/// The manifest's file name.
pub const MANIFEST: &str = "pack.toml";

/// The manifest: per id, the keys it serves. Only the subset of TOML a manifest uses: `[id]`, `name = ["…", …]`
/// (the list may run over lines), `#` comments.
pub fn parse_manifest(text: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let mut open = false;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        let err = |why: &str| format!("{}: line {}: {}", MANIFEST, n + 1, why);
        if line.is_empty() {
            continue;
        }
        let body = if open {
            line
        } else if let Some(h) = line.strip_prefix('[') {
            let id = h.strip_suffix(']').ok_or_else(|| err("a table header needs ]"))?.trim();
            if id.is_empty() || id.contains(['/', '\\', '.']) {
                return Err(err("an id is a name without . or /"));
            }
            out.push((id.to_owned(), Vec::new()));
            continue;
        } else {
            let (_, v) = line.split_once('=').ok_or_else(|| err("expected `name = [...]`"))?;
            let v = v.trim();
            v.strip_prefix('[').ok_or_else(|| err("a value is a list [\"…\", …]"))?
        };
        let cur = &mut out.last_mut().ok_or_else(|| err("a list before any [id]"))?.1;
        let (items, closed) = match body.find(']') {
            Some(i) => (&body[..i], true),
            None => (body, false),
        };
        for part in items.split(',').map(str::trim).filter(|x| !x.is_empty()) {
            let k = part.strip_prefix('"').and_then(|x| x.strip_suffix('"')).ok_or_else(|| err("a key is a quoted string"))?;
            cur.push(k.to_owned());
        }
        open = !closed;
    }
    if open {
        return Err(format!("{}: a list is not closed", MANIFEST));
    }
    Ok(out)
}

/// The manifest of a folder, as (dump name, picture file): every key of every id with the id's file.
/// No manifest, no pairs; an id without its `{id}.png` is an error.
pub fn manifest_pictures(dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let text = match std::fs::read_to_string(dir.join(MANIFEST)) {
        Ok(t) => t,
        Err(_) => return Ok(Vec::new()),
    };
    let mut out = Vec::new();
    for (id, keys) in parse_manifest(&text)? {
        let file = dir.join(format!("{}.png", id));
        if !file.is_file() {
            return Err(format!("{}: `{}` has no {}.png", MANIFEST, id, id));
        }
        for k in keys {
            out.push((format!("{}.png", k), file.clone()));
        }
    }
    Ok(out)
}

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
    // the manifest's keys, unless a file already has that name
    let mut files: Vec<(String, PathBuf)> = names.iter().map(|n| (n.clone(), dir.join(n))).collect();
    for (n, f) in manifest_pictures(dir)? {
        if is_picture(&n) && !names.contains(&n) {
            files.push((n, f));
        }
    }
    files.sort();
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for (n, path) in &files {
        let data = std::fs::read(path).map_err(|e| format!("{}: {}", n, e))?;
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

    #[test]
    fn a_manifest_gives_one_picture_to_several_keys() {
        let t = "# c\n[ok]\nkeys = [\"obj0_8x8_00000000000000aa_00000000000000bb\"]\nru = [\n  \"obj0_8x8_00000000000000aa_00000000000000cc\", # x\n  \"obj1_8x8_00000000000000dd_00000000000000ee@g.v\"\n]\n[two]\n";
        let m = parse_manifest(t).unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].0, "ok");
        assert_eq!(m[0].1.len(), 3);
        assert!(m[1].1.is_empty());
        assert!(parse_manifest("[a.b]").is_err());
        assert!(parse_manifest("k = [\"x\"]").is_err());
        assert!(parse_manifest("[a]\nk = [\"x\"").is_err());
        assert!(parse_manifest("[a]\nk = [x]").is_err());
        let dir = std::env::temp_dir().join(format!("sunpetal-p2d-m-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ok.png"), b"by id").unwrap();
        std::fs::write(dir.join("obj0_8x8_00000000000000aa_00000000000000cc.png"), b"own file").unwrap();
        std::fs::write(dir.join(MANIFEST), t.replace("[two]", "")).unwrap();
        let back = unpack(&pack_dir(&dir).unwrap()).unwrap();
        let get = |n: &str| back.iter().find(|(x, _)| x == n).map(|(_, d)| d.clone());
        assert_eq!(get("obj0_8x8_00000000000000aa_00000000000000bb.png").unwrap(), b"by id");
        assert_eq!(get("obj0_8x8_00000000000000aa_00000000000000cc.png").unwrap(), b"own file");
        assert_eq!(get("obj1_8x8_00000000000000dd_00000000000000ee@g.v.png").unwrap(), b"by id");
        assert_eq!(back.len(), 3, "the id's own file is not a key");
        std::fs::write(dir.join(MANIFEST), "[gone]\nk = [\"obj0_8x8_00000000000000aa_00000000000000bb\"]").unwrap();
        assert!(pack_dir(&dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
