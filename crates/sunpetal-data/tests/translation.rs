//! The overlay built from the translation table against the translated ROM of the translation
//! project (the files its `build_rom.py` puts in). Needs SUNPETAL_ROM (the original European ROM),
//! SUNPETAL_TRANSLATED_ROM and SUNPETAL_TABLE (`text/ru.csv`); skipped without them.
use std::path::PathBuf;

use sunpetal_data::{delta, lz10, nitrofs::NitroFs, script, translate};

fn var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

#[test]
fn the_table_makes_the_files_of_the_translated_rom() {
    let (Some(rom), Some(done), Some(table)) = (var("SUNPETAL_ROM"), var("SUNPETAL_TRANSLATED_ROM"), var("SUNPETAL_TABLE")) else {
        eprintln!("skipped: no SUNPETAL_ROM, SUNPETAL_TRANSLATED_ROM, SUNPETAL_TABLE");
        return;
    };
    let rom = std::fs::read(rom).unwrap();
    let done = std::fs::read(done).unwrap();
    let table = translate::Table::from_csv(&std::fs::read_to_string(table).unwrap(), "Русский").unwrap();
    let built = translate::build_overlay(&rom, &table, "ue").unwrap();
    for w in &built.warnings {
        eprintln!("warning: {}", w);
    }
    let (fs_rom, fs_done) = (NitroFs::parse(&rom).unwrap(), NitroFs::parse(&done).unwrap());
    let (mut same, mut different) = (0, Vec::new());
    for (path, bytes) in &built.files {
        let want = fs_done.file(&done, path).unwrap();
        let (a, b) = if path.ends_with(".z") {
            (lz10::decompress(bytes).unwrap(), lz10::decompress(want).unwrap())
        } else {
            // the translation project pads a file to its old size with zeros (its layout lock)
            let trim = |b: &[u8]| b[..b.iter().rposition(|x| *x != 0).map_or(0, |i| i + 1)].to_vec();
            (trim(bytes), trim(want))
        };
        if a == b {
            same += 1;
        } else {
            different.push(path.clone());
        }
    }
    // the files of the translated ROM that the table should have changed but did not
    let missing: Vec<&str> = fs_done
        .paths()
        .filter(|p| (p.starts_with("Chapter/Pack/") && p.ends_with(".s.z") || ["EdoComment/", "Profile/", "2D/"].iter().any(|d| p.starts_with(d)) && p.ends_with(".txt")) && fs_rom.file(&rom, p) != fs_done.file(&done, p))
        .filter(|p| !built.files.iter().any(|f| f.0 == *p))
        .collect();
    eprintln!("{} files built, {} equal, {} different, {} not built; {} entries", built.files.len(), same, different.len(), missing.len(), built.entries_changed);
    for p in different.iter().take(10) {
        eprintln!("different: {}", p);
    }
    for p in missing.iter().take(10) {
        eprintln!("not built: {}", p);
    }
    assert!(different.is_empty() && missing.is_empty());
    // what a scene's first entry reads as
    let z = built.files.iter().find(|f| f.0.ends_with("C00_01.s.z")).unwrap();
    let (_, entries) = script::parse(&script::decode_utf16(&lz10::decompress(&z.1).unwrap()).unwrap());
    assert!(entries[0].1.contains("Ладно, буду краток."));
}

/// The files the table cannot make (fonts, pictures): a delta against the original each.
#[test]
fn the_rest_of_the_translated_rom_goes_by_deltas() {
    let (Some(rom), Some(done), Some(table)) = (var("SUNPETAL_ROM"), var("SUNPETAL_TRANSLATED_ROM"), var("SUNPETAL_TABLE")) else {
        eprintln!("skipped: no SUNPETAL_ROM, SUNPETAL_TRANSLATED_ROM, SUNPETAL_TABLE");
        return;
    };
    let rom = std::fs::read(rom).unwrap();
    let done = std::fs::read(done).unwrap();
    let table = translate::Table::from_csv(&std::fs::read_to_string(table).unwrap(), "Русский").unwrap();
    let by_table: Vec<String> = translate::build_overlay(&rom, &table, "ue").unwrap().files.into_iter().map(|f| f.0).collect();
    let (fs_rom, fs_done) = (NitroFs::parse(&rom).unwrap(), NitroFs::parse(&done).unwrap());
    let mut groups: std::collections::BTreeMap<String, (usize, usize, usize)> = Default::default();
    let mut total = 0;
    for path in fs_done.paths() {
        let (a, b) = (fs_rom.file(&rom, path).unwrap(), fs_done.file(&done, path).unwrap());
        if a == b || by_table.iter().any(|p| p == path) {
            continue;
        }
        let d = delta::diff_file(path, a, b).unwrap();
        let made = delta::patch_file(path, a, &d).unwrap();
        // a `.z` file is compared unpacked; the others as they are, as the project's files are
        if path.ends_with(".z") {
            assert_eq!(lz10::decompress(&made).unwrap(), lz10::decompress(b).unwrap(), "{}", path);
        } else {
            assert_eq!(made, b, "{}", path);
        }
        let g = groups.entry(path.split('/').next().unwrap().to_string()).or_default();
        g.0 += 1;
        g.1 += d.len();
        g.2 += lz10::compress(&d).len().min(d.len());
        total += d.len();
    }
    for (k, (n, raw, packed)) in &groups {
        eprintln!("{}: {} files, deltas {} bytes ({} packed)", k, n, raw, packed);
    }
    eprintln!("total {} bytes", total);
    assert!(!groups.is_empty());
}

/// A translation through the data file and back: the overlay it makes holds the files of the
/// translated ROM (all of them, not only the text).
#[test]
fn a_translation_in_a_data_file_makes_the_translated_rom() {
    use sunpetal_data::container::Container;
    let (Some(rom), Some(done), Some(table)) = (var("SUNPETAL_ROM"), var("SUNPETAL_TRANSLATED_ROM"), var("SUNPETAL_TABLE")) else {
        eprintln!("skipped: no SUNPETAL_ROM, SUNPETAL_TRANSLATED_ROM, SUNPETAL_TABLE");
        return;
    };
    let rom = std::fs::read(rom).unwrap();
    let done = std::fs::read(done).unwrap();
    let table = translate::Table::from_csv(&std::fs::read_to_string(table).unwrap(), "Русский").unwrap();
    let (tr, problems) = translate::Translation::make(&rom, &done, table, "ru", "ue").unwrap();
    assert!(problems.is_empty(), "{:?}", problems);
    let mut c = Container::new();
    c.add(b"TRAN", tr.to_bytes().unwrap());
    let path = std::env::temp_dir().join(format!("sunpetal-tran-{}.spdata", std::process::id()));
    c.write(&path).unwrap();
    eprintln!("data file {} bytes", std::fs::metadata(&path).unwrap().len());
    let back = translate::Translation::from_bytes(&Container::read(&path).unwrap().get(b"TRAN").unwrap().data).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!((back.lang.as_str(), back.base.as_str()), ("ru", "ue"));
    let overlay = back.overlay(&rom).unwrap();
    let fs_done = NitroFs::parse(&done).unwrap();
    assert_eq!(overlay.len(), 795);
    for (p, bytes) in &overlay {
        let want = fs_done.file(&done, p).unwrap();
        let trim = |x: &[u8]| x[..x.iter().rposition(|v| *v != 0).map_or(0, |i| i + 1)].to_vec();
        if p.ends_with(".z") {
            assert_eq!(lz10::decompress(bytes).unwrap(), lz10::decompress(want).unwrap(), "{}", p);
        } else {
            assert_eq!(trim(bytes), trim(want), "{}", p);
        }
    }
    // the overlay put into the ROM gives the files of the translated one
    let mut image = rom.clone();
    sunpetal_data::nitrofs::apply_overlay(&mut image, &overlay).unwrap();
    let fs_image = NitroFs::parse(&image).unwrap();
    for (p, _) in &overlay {
        let (a, b) = (fs_image.file(&image, p).unwrap(), fs_done.file(&done, p).unwrap());
        if !p.ends_with(".z") {
            assert!(a.len() <= b.len() && b[..a.len()] == *a, "{}", p);
        }
    }
    // another ROM is refused
    let mut other = rom.clone();
    other[0x1000] ^= 1;
    assert!(back.overlay(&other).is_err());
}
