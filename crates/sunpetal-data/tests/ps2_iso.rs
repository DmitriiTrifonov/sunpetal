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

#[test]
fn materials_find_the_reference_pictures_in_the_t_blocks() {
    use sunpetal_data::ps2::{gs, lf, tblock};
    let (Some(iso), Some(export)) = (env("SUNPETAL_PS2_ISO"), env("SUNPETAL_PS2_EXPORT")) else {
        return;
    };
    let assets = export.join("../../assets");
    // role of a record, from the reference table (the scene database is a later step of the reader)
    let roles: std::collections::HashMap<(String, usize), String> = std::fs::read_to_string(assets.join("roles.tsv"))
        .unwrap()
        .lines()
        .skip(1)
        .map(|l| l.split('\t').collect::<Vec<_>>())
        .map(|c| ((c[0].to_string(), c[1].parse().unwrap()), c[4].to_string()))
        .collect();
    let mut iso = Iso::open(File::open(iso).unwrap()).unwrap();
    let mut archives = std::collections::HashMap::new();
    let mut cache: std::collections::HashMap<(String, usize), (Vec<gs::Upload>, tblock::Memory)> = Default::default();
    let png_rgba = |name: &str| read_png(&export.join("textures").join(format!("{name}.png"))).2;
    let (mut total, mut found, mut missed, mut null_ok, mut null_total) = (0, 0, Vec::new(), 0, 0);
    let mut paths: Vec<_> = std::fs::read_dir(export.join("models")).unwrap().map(|e| e.unwrap().path()).collect();
    paths.sort();
    for p in paths.iter().filter(|p| p.extension().is_some_and(|e| e == "json")) {
        let j: serde_json::Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
        let occ: Vec<(String, usize, tblock::Role)> = j["occurrences"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|o| {
                let (file, rest) = o.as_str()?.split_once('#')?;
                let idx: usize = rest.split('@').next()?.parse().ok()?;
                let role = match roles.get(&(file.to_string(), idx))?.as_str() {
                    "Scene3D" => tblock::Role::Scene3D,
                    "Chara" => tblock::Role::Chara,
                    _ => return None,
                };
                Some((file.to_string(), idx, role))
            })
            .collect();
        if occ.is_empty() {
            continue;
        }
        for (file, idx, role) in &occ {
            let key = (file.clone(), *idx);
            if cache.contains_key(&key) {
                continue;
            }
            if !archives.contains_key(file) {
                let raw = iso.file(&format!("DATA/EVENT/{file}")).unwrap();
                let arc = lf::parse(&raw).unwrap();
                archives.insert(file.clone(), (raw, arc));
            }
            let (raw, arc) = &archives[file];
            let rec = arc.data(raw, *idx).unwrap();
            let ups = tblock::uploads(rec, *role).unwrap();
            let mem = tblock::assemble(rec, &ups);
            cache.insert(key, (ups, mem));
        }
        for m in j["materials"].as_array().unwrap() {
            let prim = u64::from_str_radix(m["prim"].as_str().unwrap().trim_start_matches("0x"), 16).unwrap();
            if prim & 0x10 == 0 {
                continue;
            }
            let t = tblock::Tex0::from_bits(u64::from_str_radix(m["tex0"].as_str().unwrap().trim_start_matches("0x"), 16).unwrap());
            let mut mine = Vec::new();
            for (file, idx, _) in &occ {
                let (raw, arc) = &archives[file];
                let rec = arc.data(raw, *idx).unwrap();
                let (ups, mem) = &cache[&(file.clone(), *idx)];
                if let Some(px) = tblock::material_rgba(rec, ups, mem, &t).unwrap() {
                    mine.push(px);
                }
            }
            let mut names: Vec<String> = m["texture"].as_str().map(|s| vec![s.to_string()]).unwrap_or_default();
            names.extend(m["texture_alt"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(String::from)));
            if names.is_empty() {
                null_total += 1;
                null_ok += usize::from(mine.is_empty());
                continue;
            }
            total += 1;
            let refs: Vec<Vec<u8>> = names.iter().map(|n| png_rgba(n)).collect();
            if mine.iter().any(|px| refs.contains(px)) {
                found += 1;
            } else {
                missed.push(format!("{} {} {:?} found {}", p.file_name().unwrap().to_string_lossy(), m["texture"], t, mine.len()));
            }
        }
    }
    eprintln!(
        "{found}/{total} materials with a reference picture found; materials without one: {null_ok}/{null_total} give nothing; first misses: {:?}",
        &missed[..missed.len().min(5)]
    );
    assert!(missed.is_empty(), "{} misses", missed.len());
}

#[test]
fn gig_headers_match_the_model_catalogue() {
    use sunpetal_data::ps2::gig;
    let (Some(iso), Some(export)) = (env("SUNPETAL_PS2_ISO"), env("SUNPETAL_PS2_EXPORT")) else {
        return;
    };
    let assets = export.join("../../assets");
    let mut iso = Iso::open(File::open(iso).unwrap()).unwrap();
    let mut files: std::collections::HashMap<String, Vec<u8>> = Default::default();
    let (mut checked, mut bad) = (0, Vec::new());
    // scene and character models: id, size, object and joint counts from models.tsv
    for l in std::fs::read_to_string(assets.join("models.tsv")).unwrap().lines().skip(1) {
        let c: Vec<&str> = l.split('\t').collect();
        let (file, rest) = c[6].split_once('#').unwrap();
        let at = usize::from_str_radix(rest.split_once("@0x").unwrap().1, 16).unwrap();
        let path = load_file(&mut iso, &mut files, file);
        let d = &files[&path][at..];
        let g = gig::parse(d).unwrap_or_else(|e| panic!("{}: {e}", c[0]));
        let id = &sunpetal_data::ps2::sha1::sha1_hex(&d[..gig::size(d).unwrap()])[..10];
        let ok = id == c[0]
            && gig::size(d).unwrap() - 0x20 == c[1].parse::<usize>().unwrap()
            && g.objects.len() == c[2].parse::<usize>().unwrap()
            && g.joint_count() == c[3].parse::<usize>().unwrap()
            // 2a8972749b (an effect model) declares 2 joints but the second is overlapped by its data: the links are garbage
            && (c[0] == "2a8972749b" || g.objects.iter().all(|o| {
                let n = o.joints.len() as i16;
                o.joints.iter().all(|j| j.child < n && j.next < n && j.child >= -1 && j.next >= -1)
            }));
        if let Err(e) = check_geometry(&export, c[0], &g) {
            bad.push(format!("{}: {e}", c[0]));
        }
        checked += 1;
        if !ok {
            bad.push(format!(
                "{} id {id} size {} nobj {} joints {:?}",
                c[0],
                gig::size(d).unwrap() - 0x20,
                g.objects.len(),
                g.objects.iter().map(|o| o.joints.iter().map(|j| (j.child, j.next)).collect::<Vec<_>>()).collect::<Vec<_>>()
            ));
        }
    }
    eprintln!("{checked} scene models, {} bad: {bad:?}", bad.len());
    assert_eq!(checked, 268);
    assert!(bad.is_empty());
    // models outside the scene records: object count only (the table has no joint count)
    let mut extra = 0;
    for l in std::fs::read_to_string(assets.join("models_extra.tsv")).unwrap().lines().skip(1) {
        let c: Vec<&str> = l.split('\t').collect();
        let (file, rest) = c[5].split_once("@0x").unwrap();
        let at = usize::from_str_radix(rest, 16).unwrap();
        let path = load_file(&mut iso, &mut files, file);
        let d = &files[&path][at..];
        let g = gig::parse(d).unwrap_or_else(|e| panic!("{}: {e}", c[0]));
        assert_eq!(g.objects.len(), c[2].parse::<usize>().unwrap(), "{}", c[0]);
        check_geometry(&export, c[0], &g).unwrap_or_else(|e| panic!("{}: {e}", c[0]));
        extra += 1;
    }
    assert_eq!(checked + extra, 325);
}

fn load_file(iso: &mut Iso<File>, files: &mut std::collections::HashMap<String, Vec<u8>>, name: &str) -> String {
    // the tables name a file as "R00.BIN", "event/R03.BIN" or "data/EFFECT.BIN" (a folder per file on the disc): find it by name
    let base = name.rsplit('/').next().unwrap().to_uppercase();
    let path = iso.files().unwrap().into_iter().map(|f| f.0).find(|p| p.to_uppercase().ends_with(&format!("/{base}"))).unwrap_or_else(|| panic!("no {name} on the disc"));
    if !files.contains_key(&path) {
        files.insert(path.clone(), iso.file(&path).unwrap());
    }
    path
}

/// Series of the model against the export: counts and the whole `vtx.bin` byte for byte.
fn check_geometry(export: &std::path::Path, id: &str, g: &sunpetal_data::ps2::gig::Gig) -> Result<(), String> {
    let j: serde_json::Value = serde_json::from_slice(&std::fs::read(export.join("models").join(format!("{id}.json"))).map_err(|e| e.to_string())?).unwrap();
    let want = std::fs::read(export.join("models").join(format!("{id}.vtx.bin"))).map_err(|e| e.to_string())?;
    let all: Vec<_> = g.objects.iter().flat_map(|o| o.series.iter()).collect();
    let (nv, nt) = (all.iter().map(|s| s.verts.len()).sum::<usize>(), all.iter().map(|s| s.triangles()).sum::<usize>());
    let c = &j["counts"];
    if (c["series"].as_u64(), c["vertices"].as_u64(), c["triangles"].as_u64()) != (Some(all.len() as u64), Some(nv as u64), Some(nt as u64)) {
        return Err(format!("counts {} {nv} {nt} vs {c}", all.len()));
    }
    let mine: Vec<u8> = all.iter().flat_map(|s| s.verts.iter().flat_map(|v| v.export_bytes())).collect();
    if mine != want {
        return Err("vtx.bin differs".into());
    }
    for o in &g.objects {
        if o.series.iter().any(|s| s.bones.iter().any(|&b| usize::from(b) >= o.joints.len().max(1)) || s.verts.iter().any(|v| v.slot as usize >= s.bones.len().max(1))) {
            return Err("bone index out of range".into());
        }
    }
    for (a, b) in all.iter().zip(j["series"].as_array().unwrap()) {
        let bones: Vec<u64> = b["bones"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap()).collect();
        if a.pass as u64 != b["pass"] || a.verts.len() as u64 != b["count"] || a.bones.iter().map(|&x| u64::from(x)).collect::<Vec<_>>() != bones {
            return Err(format!("series {b}"));
        }
    }
    Ok(())
}

#[test]
fn gim_motions_match_the_export() {
    use sunpetal_data::ps2::{gim, lf, sha1::sha1_hex};
    let (Some(iso), Some(export)) = (env("SUNPETAL_PS2_ISO"), env("SUNPETAL_PS2_EXPORT")) else {
        return;
    };
    let roles = std::fs::read_to_string(export.join("../../assets/roles.tsv")).unwrap();
    let mut iso = Iso::open(File::open(iso).unwrap()).unwrap();
    let mut files = std::collections::HashMap::new();
    let mut seen = std::collections::HashSet::new();
    let mut bad = Vec::new();
    for l in roles.lines().skip(1) {
        let c: Vec<&str> = l.split('\t').collect();
        if c[4] != "MotionPack" {
            continue;
        }
        let path = load_file(&mut iso, &mut files, c[0]);
        let raw = &files[&path];
        let arc = lf::parse(raw).unwrap();
        let pk = arc.data(raw, c[1].parse().unwrap()).unwrap();
        let items = gim::pack_items(pk).unwrap();
        for &(_, off) in &items {
            if pk.get(off..off + 8) != Some(&gim::MAGIC[..]) {
                continue;
            }
            let Ok(m) = gim::parse(&pk[off..]) else { continue };
            let next = items.iter().map(|i| i.1).filter(|&x| x > off).min().unwrap_or(pk.len());
            let id = sha1_hex(&pk[off..next])[..10].to_string();
            if !seen.insert(id.clone()) {
                continue;
            }
            let j: serde_json::Value = serde_json::from_slice(&std::fs::read(export.join("motions").join(format!("{id}.json"))).unwrap()).unwrap();
            let f3 = |v: &serde_json::Value| -> Vec<f32> { v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect() };
            let mut ok = j["bone_type"] == m.bone_type
                && j["n_std"] == m.stp_table.len()
                && j["range"] == m.range
                && j["njoints"] == m.joints.len()
                && j["stp_table"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u8).collect::<Vec<_>>() == m.stp_table;
            for (a, b) in m.joints.iter().zip(j["joints"].as_array().unwrap()) {
                match a {
                    None => ok &= b.is_null(),
                    Some(a) => {
                        ok &= b["flag"] == a.flag && b["child"] == a.child && b["next"] == a.next && b["keys"].as_array().unwrap().len() == a.keys.len();
                        for (k, kj) in a.keys.iter().zip(b["keys"].as_array().unwrap()) {
                            ok &= kj["frame"] == k.frame && kj["flag"] == k.flag && f3(&kj["rot"]) == k.rot;
                            ok &= match k.pos {
                                Some(p) => f3(&kj["pos"]) == p,
                                None => kj.get("pos").is_none(),
                            };
                        }
                    }
                }
            }
            if !ok {
                bad.push(id);
            }
        }
    }
    eprintln!("{} motions, bad {bad:?}", seen.len());
    assert!(bad.is_empty());
    assert_eq!(seen.len(), 399);
}

/// У1.10: poses against `tools/ult/pose_vectors.py` (the reference `gigpose.py`): needs build/ult/pose_vectors.json.
#[test]
fn poses_match_the_reference_vectors() {
    use sunpetal_data::ps2::{gig, gim, lf, pose};
    let (Some(iso), Some(export)) = (env("SUNPETAL_PS2_ISO"), env("SUNPETAL_PS2_EXPORT")) else {
        return;
    };
    let vectors = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../build/ult/pose_vectors.json");
    let Ok(vectors) = std::fs::read(&vectors) else {
        eprintln!("skipped: no build/ult/pose_vectors.json (tools/ult/pose_vectors.py)");
        return;
    };
    let vectors: serde_json::Value = serde_json::from_slice(&vectors).unwrap();
    let assets = export.join("../../assets");
    let first = |tsv: &str, id: &str| -> String {
        std::fs::read_to_string(assets.join(tsv)).unwrap().lines().skip(1).map(|l| l.split('\t').collect::<Vec<_>>()).find(|c| c[0] == id).unwrap()[6].to_string()
    };
    let mut iso = Iso::open(File::open(iso).unwrap()).unwrap();
    let mut files = std::collections::HashMap::new();
    let (mut worst, mut frames) = (0f64, 0);
    for pair in vectors.as_array().unwrap() {
        let (mid, mo) = (pair["model"].as_str().unwrap(), pair["motion"].as_str().unwrap());
        let (mf, mrest) = first("models.tsv", mid).split_once('#').map(|(a, b)| (a.to_string(), b.to_string())).unwrap();
        let at = usize::from_str_radix(mrest.split_once("@0x").unwrap().1, 16).unwrap();
        let path = load_file(&mut iso, &mut files, &mf);
        let g = gig::parse(&files[&path][at..]).unwrap();
        let (gf, grest) = first("motions.tsv", mo).split_once('#').map(|(a, b)| (a.to_string(), b.to_string())).unwrap();
        let (idx, off) = grest.split_once("@0x").unwrap();
        let path = load_file(&mut iso, &mut files, &gf);
        let raw = &files[&path];
        let pk = lf::parse(raw).unwrap().data(raw, idx.parse().unwrap()).unwrap().to_vec();
        let m = gim::parse(&pk[usize::from_str_radix(off, 16).unwrap()..]).unwrap();
        let joints = &g.objects[0].joints;
        for f in pair["frames"].as_array().unwrap() {
            let keys = pose::sample(joints, &m, f["frame"].as_f64().unwrap(), true);
            let world = pose::world(joints, &keys);
            for (k, kj) in keys.iter().zip(f["keys"].as_array().unwrap()) {
                for (a, b) in k.iter().zip(kj.as_array().unwrap()) {
                    worst = worst.max((a - b.as_f64().unwrap()).abs());
                }
            }
            for (w, wj) in world.iter().zip(f["world"].as_array().unwrap()) {
                for (a, b) in w.iter().flatten().zip(wj.as_array().unwrap()) {
                    worst = worst.max((a - b.as_f64().unwrap()).abs());
                }
            }
            frames += 1;
        }
    }
    eprintln!("{} pairs, {frames} frames, worst deviation {worst:e}", vectors.as_array().unwrap().len());
    assert!(worst < 1e-4);
    assert_eq!(vectors.as_array().unwrap().len(), 20);
}

#[test]
fn scene_databases_give_the_reference_roles_and_edges() {
    use sunpetal_data::ps2::{lf, scenedb};
    let (Some(iso), Some(export)) = (env("SUNPETAL_PS2_ISO"), env("SUNPETAL_PS2_EXPORT")) else {
        return;
    };
    let assets = export.join("../../assets");
    let table =
        |name: &str| -> Vec<Vec<String>> { std::fs::read_to_string(assets.join(name)).unwrap().lines().skip(1).map(|l| l.split('\t').map(String::from).collect()).collect() };
    let roles_ref = table("roles.tsv");
    let mut names: Vec<String> = roles_ref.iter().map(|r| r[0].clone()).collect();
    names.dedup();
    let mut iso = Iso::open(File::open(iso).unwrap()).unwrap();
    let mut files = std::collections::HashMap::new();
    let (mut roles_mine, mut edges_mine) = (Vec::new(), Vec::new());
    for name in &names {
        let path = load_file(&mut iso, &mut files, name);
        let raw = &files[&path];
        let arc = lf::parse(raw).unwrap();
        let db = scenedb::parse_db(arc.data(raw, 0).unwrap()).unwrap();
        for (i, r) in scenedb::roles(&db, arc.records.len()).into_iter().enumerate() {
            roles_mine.push(vec![name.clone(), i.to_string(), arc.records[i].at.to_string(), arc.records[i].size.to_string(), r]);
        }
        edges_mine.extend(scenedb::edges(name, raw).unwrap().into_iter().map(Vec::from));
    }
    roles_mine.sort();
    let mut want = roles_ref.clone();
    want.sort();
    assert_eq!(roles_mine, want, "roles");
    let mut edges_ref: Vec<Vec<String>> = table("edges.tsv");
    for e in &mut edges_ref {
        e.resize(4, String::new());
    }
    edges_ref.sort();
    edges_mine.sort();
    eprintln!("{} scenes, {} roles, {} edges", names.len(), roles_mine.len(), edges_mine.len());
    let missing: Vec<_> = edges_ref.iter().filter(|e| edges_mine.binary_search(e).is_err()).take(5).collect();
    let extra: Vec<_> = edges_mine.iter().filter(|e| edges_ref.binary_search(e).is_err()).take(5).collect();
    assert!(missing.is_empty() && extra.is_empty(), "missing {missing:?} extra {extra:?}");
    assert_eq!((names.len(), edges_mine.len()), (20, 4270));
}

#[test]
fn events_disassemble_and_their_calls_match_the_export() {
    use sunpetal_data::ps2::{events, lf, scenedb};
    let (Some(iso), Some(export)) = (env("SUNPETAL_PS2_ISO"), env("SUNPETAL_PS2_EXPORT")) else {
        return;
    };
    let roles = std::fs::read_to_string(export.join("../../assets/roles.tsv")).unwrap();
    let mut names: Vec<&str> = roles.lines().skip(1).map(|l| l.split('\t').next().unwrap()).collect();
    names.dedup();
    let mut iso = Iso::open(File::open(iso).unwrap()).unwrap();
    let mut files = std::collections::HashMap::new();
    let (mut total, mut bad_ops, mut no_end, mut ncalls) = (0, 0, 0, 0);
    for name in names {
        let path = load_file(&mut iso, &mut files, name);
        let raw = &files[&path];
        let arc = lf::parse(raw).unwrap();
        let rec0 = arc.data(raw, 0).unwrap();
        let db = scenedb::parse_db(rec0).unwrap();
        let mut mine = Vec::new();
        for (ei, words) in events::static_events(rec0, &db).unwrap() {
            total += 1;
            let mut ops = events::dis(&words, 0, 20000, false);
            while matches!(ops.last(), Some((_, events::Op::Bad(0)))) {
                ops.pop();
            }
            if !matches!(ops.last(), Some((_, events::Op::End | events::Op::Simple(0x1B)))) {
                no_end += 1;
            }
            bad_ops += ops.iter().filter(|o| matches!(o.1, events::Op::Bad(_))).count();
            mine.extend(events::calls(ei, &ops));
        }
        let stem = name.trim_end_matches(".BIN");
        let jsonl = std::fs::read_to_string(export.join("events").join(format!("{stem}.jsonl"))).unwrap();
        let want: Vec<serde_json::Value> = jsonl.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(mine.len(), want.len(), "{name}: calls");
        for (a, b) in mine.iter().zip(&want) {
            let args_ok = a.args.len() == b["args"].as_array().unwrap().len()
                && a.args.iter().zip(b["args"].as_array().unwrap()).all(|(x, y)| match (x, y) {
                    (None, y) => y.is_null(),
                    (Some(events::Value::Int(i)), y) => y.as_i64() == Some(*i),
                    (Some(events::Value::Float(f)), y) => y.as_f64().map(|v| v as f32) == Some(*f as f32), // serde_json parses doubles inexactly
                });
            let ok = b["event"] == a.event
                && b["pc"] == a.pc
                && b["seq"] == a.seq
                && b["x"] == a.x
                && b["m"] == a.m
                && b["method"].as_str().and_then(|s| s.parse::<u16>().ok()) == Some(a.method)
                && b["class"] == events::class_name(a.class).map_or(a.class.to_string(), String::from)
                && args_ok;
            assert!(ok, "{name}: {a:?} vs {b}");
        }
        ncalls += mine.len();
    }
    eprintln!("{total} events, {ncalls} calls, unknown ops {bad_ops}, without end {no_end}");
    assert_eq!((bad_ops, no_end), (0, 0));
    assert_eq!(total, 1384);
}

#[test]
fn mesh_triangles_add_up_to_the_exports_counts() {
    use sunpetal_data::ps2::{gig, mesh};
    let (Some(iso), Some(export)) = (env("SUNPETAL_PS2_ISO"), env("SUNPETAL_PS2_EXPORT")) else {
        return;
    };
    let assets = export.join("../../assets");
    let mut iso = Iso::open(File::open(iso).unwrap()).unwrap();
    let mut files = std::collections::HashMap::new();
    let (mut models, mut degenerate) = (0, 0);
    for (tsv, col) in [("models.tsv", 6), ("models_extra.tsv", 5)] {
        for l in std::fs::read_to_string(assets.join(tsv)).unwrap().lines().skip(1) {
            let c: Vec<&str> = l.split('\t').collect();
            let (file, rest) = c[col].split_once("@0x").map_or((c[col], ""), |(a, b)| (a, b));
            let (file, at) = if col == 6 { (file.split('#').next().unwrap(), rest) } else { (file, rest) };
            let at = usize::from_str_radix(at, 16).unwrap();
            let path = load_file(&mut iso, &mut files, file);
            let d = &files[&path];
            let model = &d[at..at + gig::size(&d[at..]).unwrap()];
            let m = mesh::entry_mesh(model);
            let j: serde_json::Value = serde_json::from_slice(&std::fs::read(export.join("models").join(format!("{}.json", c[0]))).unwrap()).unwrap();
            assert_eq!((m.triangles.len() + m.degenerate as usize) as u64, j["counts"]["triangles"].as_u64().unwrap(), "{}", c[0]);
            degenerate += m.degenerate;
            models += 1;
        }
    }
    eprintln!("{models} models, {degenerate} degenerate triangles dropped");
    assert_eq!(models, 325);
}

#[test]
fn anim_section_holds_every_motion_and_round_trips() {
    use sunpetal_data::anim::{key_from_hex, Anim};
    use sunpetal_data::ps2::{gim, lf, sha1::sha1_hex};
    let (Some(iso), Some(export)) = (env("SUNPETAL_PS2_ISO"), env("SUNPETAL_PS2_EXPORT")) else {
        return;
    };
    let roles = std::fs::read_to_string(export.join("../../assets/roles.tsv")).unwrap();
    let mut iso = Iso::open(File::open(iso).unwrap()).unwrap();
    let mut files = std::collections::HashMap::new();
    let mut anim = Anim::default();
    for l in roles.lines().skip(1) {
        let c: Vec<&str> = l.split('\t').collect();
        if c[4] != "MotionPack" {
            continue;
        }
        let path = load_file(&mut iso, &mut files, c[0]);
        let raw = &files[&path];
        let pk = lf::parse(raw).unwrap().data(raw, c[1].parse().unwrap()).unwrap().to_vec();
        let items = gim::pack_items(&pk).unwrap();
        for &(id, off) in &items {
            if pk.get(off..off + 8) != Some(&gim::MAGIC[..]) {
                continue;
            }
            let Ok(m) = gim::parse(&pk[off..]) else { continue };
            let next = items.iter().map(|i| i.1).filter(|&x| x > off).min().unwrap_or(pk.len());
            let key = key_from_hex(&sha1_hex(&pk[off..next])[..10]).unwrap();
            if !anim.motions.iter().any(|x| x.0 == key) {
                anim.motions.push((key, m));
            }
            anim.ids.push((c[0].to_string(), id, key));
        }
    }
    let bytes = anim.encode();
    eprintln!("{} motions, {} ids, {} bytes", anim.motions.len(), anim.ids.len(), bytes.len());
    assert_eq!(anim.motions.len(), 399);
    eprintln!("lz10: {} bytes", sunpetal_data::lz10::compress(&bytes).len());
    assert_eq!(Anim::decode(&bytes).unwrap(), anim);
}
