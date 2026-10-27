//! Puts the DS ROM into a data file (the `NDS ` section), so that the game starts from the data
//! file alone. For the player's machine: the file holds the game and is never shared.
//!
//!     sunpetal-pack-rom --rom GAME.nds --out sunpetal.data [--merge OTHER.spdata]
//!
//! With `--merge` the other sections of that file are kept (pictures, translations).
use std::path::PathBuf;
use std::process::ExitCode;

use sunpetal_data::container::Container;
use sunpetal_data::delta::crc32;
use sunpetal_data::dsrom;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}", e);
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let (mut rom, mut out, mut merge) = (None, None, None);
    while let Some(a) = args.next() {
        let mut v = || args.next().ok_or_else(|| format!("{} wants a value", a));
        match a.as_str() {
            "--rom" => rom = Some(PathBuf::from(v()?)),
            "--out" => out = Some(PathBuf::from(v()?)),
            "--merge" => merge = Some(PathBuf::from(v()?)),
            _ => return Err(format!("unknown argument {}", a)),
        }
    }
    let (Some(rom), Some(out)) = (rom, out) else {
        return Err("usage: sunpetal-pack-rom --rom GAME.nds --out sunpetal.data [--merge OTHER.spdata]".into());
    };
    let image = std::fs::read(&rom).map_err(|e| format!("{}: {}", rom.display(), e))?;
    let code: [u8; 4] = image.get(0xC..0x10).ok_or("the ROM is too short")?.try_into().unwrap();
    if &code != b"YHNP" {
        return Err(format!("{}: not the supported ROM (game code {:?}, expected YHNP)", rom.display(), String::from_utf8_lossy(&code)));
    }
    let mut c = match &merge {
        Some(p) => Container::read(p)?,
        None => Container::new(),
    };
    let crc = crc32(&image);
    // a merged file's translations are for one ROM: the same one
    if let Some(m) = c.meta()? {
        m.check_rom(crc)?;
    }
    c.remove(dsrom::TAG);
    c.add(dsrom::TAG, dsrom::pack(&image)?);
    c.set_rom(code, crc);
    c.write(&out)?;
    println!("{}: ROM {} bytes, file {} bytes", out.display(), image.len(), std::fs::metadata(&out).map_or(0, |m| m.len()));
    Ok(())
}
