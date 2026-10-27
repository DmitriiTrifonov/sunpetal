//! Makes a translation section for the data file from a translation project: the table of texts and
//! the translated ROM, both against the original ROM. For the author of a translation; the player
//! gets the data file and never needs the translated ROM.
//!
//!     sunpetal-pack-translation --rom ORIGINAL.nds [--translated TRANSLATED.nds] --table text/ru.csv \
//!         --lang ru [--column Русский] [--base ue] --out sunpetal.data [--merge OTHER.spdata]
//!
//! With `--merge` the sections of that file are kept (the pictures), the translation of the same
//! language in it is replaced.
use std::path::PathBuf;
use std::process::ExitCode;

use sunpetal_data::container::Container;
use sunpetal_data::translate::{Table, Translation};

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
    let (mut rom, mut done, mut table, mut out, mut merge) = (None, None, None, None, None);
    let (mut lang, mut column, mut base) = ("ru".to_string(), "Русский".to_string(), "ue".to_string());
    while let Some(a) = args.next() {
        let mut v = || args.next().ok_or_else(|| format!("{} wants a value", a));
        match a.as_str() {
            "--rom" => rom = Some(PathBuf::from(v()?)),
            "--translated" => done = Some(PathBuf::from(v()?)),
            "--table" => table = Some(PathBuf::from(v()?)),
            "--out" => out = Some(PathBuf::from(v()?)),
            "--merge" => merge = Some(PathBuf::from(v()?)),
            "--lang" => lang = v()?,
            "--column" => column = v()?,
            "--base" => base = v()?,
            _ => return Err(format!("unknown argument {}", a)),
        }
    }
    let (Some(rom), Some(table), Some(out)) = (rom, table, out) else {
        return Err("usage: sunpetal-pack-translation --rom ORIGINAL.nds [--translated TRANSLATED.nds] --table ru.csv --lang ru --out sunpetal.data".into());
    };
    let read = |p: &PathBuf| std::fs::read(p).map_err(|e| format!("{}: {}", p.display(), e));
    let table = Table::from_csv(&String::from_utf8(read(&table)?).map_err(|e| e.to_string())?, &column)?;
    // without a translated ROM there is no graphics and font to carry: the texts alone
    let rom = read(&rom)?;
    let compare = done.is_some();
    let done = match &done {
        Some(p) => read(p)?,
        None => rom.clone(),
    };
    let (tr, problems) = Translation::make(&rom, &done, table, &lang, &base)?;
    // with no translated ROM there is nothing to compare the texts with
    let problems = if compare { problems } else { Vec::new() };
    for p in &problems {
        eprintln!("warning: {}", p);
    }
    let bytes = tr.to_bytes()?;
    let mut c = match &merge {
        Some(p) => Container::read(p)?,
        None => Container::new(),
    };
    c.sections.retain(|s| &s.tag != b"TRAN" || Translation::from_bytes(&s.data).map_or(true, |t| t.lang != lang));
    c.add(b"TRAN", bytes.clone());
    c.set_rom(rom[0xC..0x10].try_into().map_err(|_| "the ROM is too short")?, tr.rom_crc);
    c.write(&out)?;
    println!("{}: {} texts in {} files, {} deltas, section {} bytes, {} problems", lang, tr.table.texts(), tr.table.files(), tr.deltas.len(), bytes.len(), problems.len());
    Ok(())
}
