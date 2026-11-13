//! The PS2 disc's file system against the reference export (docs/ULTIMATE-IMPORT.md, У1.1).
//! Needs SUNPETAL_PS2_ISO (the English v1.0 disc: the export was made from it, the Japanese one differs in the
//! patched pages, e.g. NAKA block 16_223) and SUNPETAL_PS2_EXPORT (`sunpetal-ee/out/export/v1`); skipped without them.
use std::fs::File;
use std::path::PathBuf;
use sunpetal_data::ps2::{iso::Iso, sha1::sha1_hex};

fn env(name: &str) -> Option<PathBuf> {
    let v = std::env::var_os(name);
    if v.is_none() {
        eprintln!("skipped: no {name}");
    }
    v.map(PathBuf::from)
}

fn field(manifest: &str, key: &str) -> String {
    let at = manifest.find(&format!("\"{key}\"")).expect("key");
    manifest[at..].split('"').nth(3).expect("value").to_string()
}

#[test]
fn the_disc_has_44_files_and_the_executable_matches_the_export() {
    let (Some(iso), Some(export)) = (env("SUNPETAL_PS2_ISO"), env("SUNPETAL_PS2_EXPORT")) else {
        return;
    };
    let manifest = std::fs::read_to_string(export.join("manifest.json")).unwrap();
    let mut iso = Iso::open(File::open(iso).unwrap()).unwrap();
    let files = iso.files().unwrap();
    assert_eq!(files.len(), 44, "{:?}", files.iter().map(|f| &f.0).collect::<Vec<_>>());
    let elf = iso.file("SLPS_250.34").unwrap();
    assert_eq!(sha1_hex(&elf), field(&manifest, "elf_sha1"));
}

#[test]
fn the_lf_archives_hold_their_records_end_to_end() {
    let Some(iso) = env("SUNPETAL_PS2_ISO") else {
        return;
    };
    let mut iso = Iso::open(File::open(iso).unwrap()).unwrap();
    let (mut archives, mut records, mut loose) = (0, 0, Vec::new());
    for (path, e) in iso.files().unwrap() {
        // the plan counts the data archives: not the empty VBST.BIN nor the sound and video (MOVIE, PCM0-2)
        if path.contains("PCM") || path.contains("MOVIE") || path.contains("VBST") {
            continue;
        }
        let raw = iso.read(e.lba, e.size).unwrap();
        if raw.len() < 2 || &raw[..2] != b"LF" {
            continue;
        }
        let a = sunpetal_data::ps2::lf::parse(&raw).unwrap_or_else(|err| panic!("{path}: {err}"));
        archives += 1;
        records += a.records.len();
        if a.tail != 0 || !a.ids_follow_sectors() {
            loose.push(format!("{path}: tail {} ids {}", a.tail, a.ids_follow_sectors()));
        }
    }
    eprintln!("{archives} archives, {records} records; loose: {loose:?}");
    assert_eq!((archives, records), (31, 1717));
}

#[test]
fn bpe_gives_the_reference_bytes_of_271_naka_blocks_and_6_yoshi_records() {
    let (Some(iso), Some(export)) = (env("SUNPETAL_PS2_ISO"), env("SUNPETAL_PS2_EXPORT")) else {
        return;
    };
    use sunpetal_data::ps2::{bpe, lf};
    let reference = export.join("../../naka/bpe");
    let mut iso = Iso::open(File::open(iso).unwrap()).unwrap();
    let raw = iso.file("DATA/NAKA/NAKA.BIN").unwrap();
    let arc = lf::parse(&raw).unwrap();
    let mut blocks: Vec<(String, &[u8])> = (20..48).map(|i| (i.to_string(), arc.data(&raw, i).unwrap())).collect();
    let b16 = arc.data(&raw, 16).unwrap();
    let u32at = |b: &[u8], p: usize| u32::from_le_bytes(b[p..p + 4].try_into().unwrap()) as usize;
    for k in 0..u32at(b16, 0) {
        let (off, sz) = (u32at(b16, 4 + 8 * k), u32at(b16, 8 + 8 * k));
        let r = &b16[off..off + sz];
        if sz >= 8 && u32at(r, 4) == sz - 8 {
            blocks.push((format!("16_{k}"), r));
        }
    }
    assert_eq!(blocks.len(), 271);
    for (name, rec) in &blocks {
        let want = std::fs::read(reference.join(format!("{name}.bin"))).unwrap();
        assert_eq!(bpe::decode(rec).unwrap_or_else(|e| panic!("{name}: {e}")), want, "block {name}");
    }
    let raw = iso.file("DATA/YOSHI/YOSHI.BIN").unwrap();
    let arc = lf::parse(&raw).unwrap();
    assert_eq!(arc.records.len(), 6);
    for i in 0..6 {
        let rec = arc.data(&raw, i).unwrap();
        let out = bpe::decode(rec).unwrap_or_else(|e| panic!("YOSHI {i}: {e}"));
        assert_eq!(out.len(), u32at(rec, 0));
    }
}

fn read_png(path: &std::path::Path) -> (u32, u32, Vec<u8>) {
    let mut r = png::Decoder::new(File::open(path).unwrap()).read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).unwrap();
    assert_eq!((info.color_type, info.bit_depth), (png::ColorType::Rgba, png::BitDepth::Eight));
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

#[test]
fn gs_uploads_decode_to_the_reference_pictures_of_the_export() {
    use sunpetal_data::ps2::{gs, lf};
    let (Some(iso), Some(export)) = (env("SUNPETAL_PS2_ISO"), env("SUNPETAL_PS2_EXPORT")) else {
        return;
    };
    let tex = export.join("../../assets/tex");
    let mut want: std::collections::BTreeSet<String> =
        std::fs::read_dir(&tex).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().trim_end_matches(".png").to_string()).collect();
    let total = want.len();
    let mut iso = Iso::open(File::open(iso).unwrap()).unwrap();
    let (mut checked, mut bad, mut seen) = (0, Vec::new(), std::collections::BTreeSet::new());
    for (path, e) in iso.files().unwrap() {
        if ["PCM", "MOVIE", "VBST", "SLPS"].iter().any(|s| path.contains(s)) {
            continue;
        }
        let raw = iso.read(e.lba, e.size).unwrap();
        let Ok(arc) = lf::parse(&raw) else { continue };
        for rec in &arc.records {
            let ups = gs::uploads(&raw, rec.range());
            for (i, im) in ups.iter().enumerate() {
                if im.is_clut() {
                    continue;
                }
                let clut = if matches!(im.psm, gs::PSM_T8 | gs::PSM_T4) {
                    ups[i + 1..].iter().take_while(|u| u.is_clut()).find(|u| u.psm == gs::PSM_CT32 && u.w * u.h == if im.psm == gs::PSM_T8 { 256 } else { 16 })
                } else {
                    None
                };
                let hash = |r: std::ops::Range<usize>| sha1_hex(&raw[r]);
                let name = format!("{}_{}", &hash(im.data.clone())[..10], clut.map_or("-".to_string(), |c| hash(c.data.clone())[..8].to_string()));
                if !want.contains(&name) || !seen.insert(name.clone()) {
                    continue;
                }
                let (w, h, px) = read_png(&tex.join(format!("{name}.png")));
                let ours = gs::to_rgba(&raw, im, clut);
                checked += 1;
                if ours.as_deref().ok() != Some(&px[..]) || (w, h) != (im.w, im.h) {
                    bad.push(format!("{name} {path}#{} psm {:#x} {}x{}: {:?}", rec.at, im.psm, im.w, im.h, ours.err()));
                }
            }
        }
    }
    for n in &seen {
        want.remove(n);
    }
    eprintln!(
        "{total} reference pictures, {checked} decoded, {} wrong, {} not found: {:?} {:?}",
        bad.len(),
        want.len(),
        &bad[..bad.len().min(5)],
        want.iter().take(5).collect::<Vec<_>>()
    );
    assert!(bad.is_empty());
    assert!(want.is_empty(), "{} of {total} pictures not found", want.len());
}
