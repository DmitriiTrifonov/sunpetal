//! Tools for translators, on their own copy of the ROM (docs/TRANSLATING.md).
//!
//!     sunpetal-translate export --rom GAME.nds --lang Name [--base ue] [--from FILLED.csv --column Name] [--out template.csv]
//!     sunpetal-translate check  --rom GAME.nds --table FILE.csv --column Name [--base ue] [--font FILE.nftr] [--max-width 188] [--show 30]
//!     sunpetal-translate progress --rom GAME.nds --table FILE.csv --column Name [--base ue]
//!
//! `export` writes a table with the original text and an empty column to translate; with `--from`
//! the translations of an older table are taken into the new one. `check` finds texts the ROM has
//! no place for, lines that are too long or too many for the dialogue window, and characters the
//! font lacks (the font is the ROM's, or the translated one with `--font`). It exits with 1 if
//! there is a problem. The template holds the game's text: it is not for sharing.
use std::path::PathBuf;
use std::process::ExitCode;

use sunpetal_data::nftr::Font;
use sunpetal_data::nitrofs::NitroFs;
use sunpetal_data::tools;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {}", e);
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().ok_or("usage: sunpetal-translate export|check|progress ... (see the top of the program's source)")?;
    let (mut rom, mut table, mut from, mut out, mut font) = (None, None, None, None, None);
    let (mut lang, mut column, mut base) = (String::new(), String::new(), "ue".to_string());
    let (mut max_width, mut show) = (tools::MAX_WIDTH, 30usize);
    while let Some(a) = args.next() {
        let mut v = || args.next().ok_or_else(|| format!("{} wants a value", a));
        match a.as_str() {
            "--rom" => rom = Some(PathBuf::from(v()?)),
            "--table" => table = Some(PathBuf::from(v()?)),
            "--from" => from = Some(PathBuf::from(v()?)),
            "--out" => out = Some(PathBuf::from(v()?)),
            "--font" => font = Some(PathBuf::from(v()?)),
            "--lang" => lang = v()?,
            "--column" => column = v()?,
            "--base" => base = v()?,
            "--max-width" => max_width = v()?.parse().map_err(|_| "--max-width wants a number")?,
            "--show" => show = v()?.parse().map_err(|_| "--show wants a number")?,
            _ => return Err(format!("unknown argument {}", a)),
        }
    }
    let read = |p: &PathBuf| std::fs::read(p).map_err(|e| format!("{}: {}", p.display(), e));
    let rom = read(&rom.ok_or("--rom is needed")?)?;
    match cmd.as_str() {
        "export" => {
            if lang.is_empty() {
                return Err("--lang NAME is needed: the title of the column to translate into".into());
            }
            let all = tools::entries(&rom, &base)?;
            let filled = match &from {
                Some(p) => Some(String::from_utf8(read(p)?).map_err(|e| e.to_string())?),
                None => None,
            };
            let csv = tools::template_csv(&all, &lang, filled.as_deref().map(|t| (t, if column.is_empty() { lang.as_str() } else { column.as_str() })));
            let out = out.unwrap_or_else(|| PathBuf::from("template.csv"));
            std::fs::write(&out, csv).map_err(|e| format!("{}: {}", out.display(), e))?;
            println!("{}: {} texts", out.display(), all.len());
            Ok(ExitCode::SUCCESS)
        }
        "check" | "progress" => {
            let csv = String::from_utf8(read(&table.ok_or("--table is needed")?)?).map_err(|e| e.to_string())?;
            if column.is_empty() {
                return Err("--column NAME is needed".into());
            }
            if cmd == "progress" {
                let (mut all, mut done) = (0, 0);
                for (chap, (n, d)) in tools::progress(&rom, &base, &csv, &column)? {
                    println!("{:<12} {:>5} of {:>5}", chap, d, n);
                    all += n;
                    done += d;
                }
                println!("{:<12} {:>5} of {:>5} ({:.1} %)", "all", done, all, 100.0 * done as f64 / all.max(1) as f64);
                return Ok(ExitCode::SUCCESS);
            }
            let font = match &font {
                Some(p) => Font::parse(&read(p)?)?,
                None => {
                    let fs = NitroFs::parse(&rom)?;
                    Font::parse(fs.file(&rom, tools::DIALOGUE_FONT).ok_or("the ROM has no dialogue font")?)?
                }
            };
            let rep = tools::check(&rom, &base, &csv, &column, &font, max_width)?;
            for p in rep.problems.iter().take(show) {
                println!("  {}", p);
            }
            if rep.problems.len() > show {
                println!("  ... and {} more", rep.problems.len() - show);
            }
            println!("{} rows, {} translated, {} of the ROM's texts still without; {} problems", rep.rows, rep.translated, rep.missing, rep.problems.len());
            Ok(if rep.problems.is_empty() { ExitCode::SUCCESS } else { ExitCode::from(1) })
        }
        _ => Err(format!("unknown command {} (export, check, progress)", cmd)),
    }
}
