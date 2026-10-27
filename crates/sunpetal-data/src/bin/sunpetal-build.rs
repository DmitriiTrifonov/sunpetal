//! The data builder: makes `sunpetal.data` from the player's own copies of the game.
//!
//!     sunpetal-build --rom GAME.nds [--disc GAME.iso] [--textures PACK.spdata] [--translation FILE.spdata]...
//!                    [--out sunpetal.data] [--cache DIR] [--dump DIR] [--tools DIR] [--python python3]
//!
//! The ROM goes in as it is (the `NDS ` section). With `--disc` the PS2 disc's pictures are matched
//! with the DS textures by `tools/build-data.py` (a quarter of an hour the first time; the result is
//! kept in `--cache` by the ROM and the disc, and used again). `--textures` takes a texture file
//! already made. Each `--translation` file brings its translations in (they must be for this ROM).
//! A part left out is not in the file. The result is whole or not there, and carries `META`.
//! The file holds the game: it is for the machine it is built on and never shared.
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Instant;

use sunpetal_data::container::{verify_file, Container};
use sunpetal_data::delta::crc32;
use sunpetal_data::dsrom;
use sunpetal_data::translate::Translation;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn say(s: &str) {
    println!("{}", s);
}

/// `name-size-mtime`: what tells a disc or a ROM apart without reading gigabytes.
fn stamp(p: &Path) -> Result<String, String> {
    let m = std::fs::metadata(p).map_err(|e| format!("{}: {}", p.display(), e))?;
    let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    Ok(format!("{}-{}", m.len(), t))
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let (mut rom, mut disc, mut textures, mut out, mut cache, mut dump) = (None, None, None, PathBuf::from("sunpetal.data"), None, None);
    let (mut tools, mut python) = (PathBuf::from("tools"), "python3".to_string());
    let mut translations: Vec<PathBuf> = Vec::new();
    while let Some(a) = args.next() {
        let mut v = || args.next().ok_or_else(|| format!("{} wants a value", a));
        match a.as_str() {
            "--rom" => rom = Some(PathBuf::from(v()?)),
            "--disc" => disc = Some(PathBuf::from(v()?)),
            "--textures" => textures = Some(PathBuf::from(v()?)),
            "--translation" => translations.push(PathBuf::from(v()?)),
            "--out" => out = PathBuf::from(v()?),
            "--cache" => cache = Some(PathBuf::from(v()?)),
            "--dump" => dump = Some(PathBuf::from(v()?)),
            "--tools" => tools = PathBuf::from(v()?),
            "--python" => python = v()?,
            _ => return Err(format!("unknown argument {}", a)),
        }
    }
    let Some(rom_path) = rom else {
        return Err("usage: sunpetal-build --rom GAME.nds [--disc GAME.iso] [--textures PACK.spdata] [--translation FILE.spdata]... [--out sunpetal.data]".into());
    };
    if disc.is_some() && textures.is_some() {
        return Err("--disc and --textures both give the textures: take one".into());
    }
    let t0 = Instant::now();

    say(&format!("ROM: {}", rom_path.display()));
    let image = std::fs::read(&rom_path).map_err(|e| format!("{}: {}", rom_path.display(), e))?;
    let code: [u8; 4] = image.get(0xC..0x10).ok_or("the ROM is too short")?.try_into().unwrap();
    if &code != b"YHNP" {
        return Err(format!("{}: not the supported ROM (game code {:?}; the game's European release, YHNP, is the one)", rom_path.display(), String::from_utf8_lossy(&code)));
    }
    let crc = crc32(&image);
    say(&format!("  game code YHNP, CRC-32 {:08X}, {} bytes", crc, image.len()));

    let mut c = Container::new();
    c.add(dsrom::TAG, dsrom::pack(&image)?);

    // the textures: made from the disc (kept by the ROM and the disc), or taken ready
    let pack = if let Some(disc) = &disc {
        let cache = cache.clone().unwrap_or_else(|| PathBuf::from("build/cache"));
        std::fs::create_dir_all(&cache).map_err(|e| format!("{}: {}", cache.display(), e))?;
        let made = cache.join(format!("textures-{:08X}-{}.spdata", crc, stamp(disc)?));
        if made.is_file() && verify_file(&made).is_ok() {
            say(&format!("PS2 textures: from the cache {}", made.display()));
        } else {
            say(&format!("PS2 textures: matching {} with the ROM (a long time the first time)", disc.display()));
            let script = tools.join("build-data.py");
            if !script.is_file() {
                return Err(format!("{} not found (--tools DIR names the folder with build-data.py)", script.display()));
            }
            let part = made.with_extension("part");
            let mut cmd = Command::new(&python);
            cmd.arg(&script).arg(&rom_path).arg(disc).arg(&part).arg("--cache").arg(cache.join("disc-index.pickle"));
            if let Some(d) = &dump {
                cmd.arg("--dump").arg(d);
            }
            let st = cmd.status().map_err(|e| format!("{}: {}", python, e))?;
            if !st.success() {
                return Err("build-data.py failed (see above)".into());
            }
            std::fs::rename(&part, &made).map_err(|e| e.to_string())?;
        }
        Some(made)
    } else {
        textures
    };
    if let Some(p) = pack {
        let t = Container::read(&p)?;
        let n = t.all(b"TEXS").count();
        if n == 0 {
            return Err(format!("{} has no textures", p.display()));
        }
        say(&format!("  {} texture section from {}", n, p.display()));
        c.sections.extend(t.sections.into_iter().filter(|s| &s.tag == b"TEXS"));
    } else {
        say("PS2 textures: none (no --disc or --textures)");
    }

    for p in &translations {
        let t = Container::read_only(p, |t| t == b"TRAN" || t == b"META")?;
        if let Some(m) = t.meta()? {
            m.check_rom(crc).map_err(|e| format!("{}: {}", p.display(), e))?;
        }
        let mut n = 0;
        for s in t.all(b"TRAN") {
            let tr = Translation::from_bytes(&s.data).map_err(|e| format!("{}: {}", p.display(), e))?;
            if tr.rom_crc != crc {
                return Err(format!("{}: translation `{}` is for another ROM (CRC-32 {:08X}; this one is {:08X})", p.display(), tr.lang, tr.rom_crc, crc));
            }
            // a later file wins for a language
            c.sections.retain(|x| &x.tag != b"TRAN" || Translation::from_bytes(&x.data).map_or(true, |o| o.lang != tr.lang));
            say(&format!("  translation `{}` ({} texts, {} bytes) from {}", tr.lang, tr.table.texts(), s.data.len(), p.display()));
            c.add(b"TRAN", s.data.clone());
            n += 1;
        }
        if n == 0 {
            return Err(format!("{} has no translation", p.display()));
        }
    }
    if translations.is_empty() {
        say("translations: none (no --translation)");
    }

    c.set_rom(code, crc);
    c.write(&out)?;
    // read back as the game will: whole, and each part as recorded
    let meta = verify_file(&out)?.ok_or("the file written has no META")?;
    let size = std::fs::metadata(&out).map_or(0, |m| m.len());
    say(&format!("{}: {} parts, {} bytes, checked ({:.1} s)", out.display(), meta.parts.len(), size, t0.elapsed().as_secs_f32()));
    Ok(())
}
