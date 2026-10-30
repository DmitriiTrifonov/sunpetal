//! Makes the game's font (NFTR) from a TrueType/OpenType font for the characters of a translation.
//! For the author of a translation; the font file is theirs and does not go into the repository.
//!
//!     sunpetal-font --rom GAME.nds --ttf FONT.ttf --table text/ru.csv [--column Русский] \
//!         [--all DIR] [--pixel] [--size 12] [--weight 1.0] [--show "Привет, мир"] --out font.nftr
//!
//! `--pixel` makes it look like the game's own drawn font: one-pixel strokes, no blur (size 13 and
//! weight 1.5 unless given).
//!
//! `--all DIR` makes all four fonts of the game (the dialogue one and the three of the 2D screens) into
//! DIR, for `sunpetal-pack-translation --font-dir DIR`.
//!
//! The shape (cell, baseline, bits per pixel) is taken from the ROM's dialogue font
//! `Chapter/Pack/Common/font.nftr` (`--base-font FILE.nftr` for another). `--show` prints a line
//! as pixels, to choose the size and the weight by eye. Check the width of the lines with
//! `sunpetal-translate check --font font.nftr`.
use std::path::PathBuf;
use std::process::ExitCode;

use sunpetal_data::fontgen::{self, Params};
use sunpetal_data::nftr::Nftr;
use sunpetal_data::nitrofs::NitroFs;
use sunpetal_data::translate::{font_file_name, parse_csv, DIALOGUE_FONT, FONTS};


fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let (mut rom, mut ttf, mut table, mut out, mut base_font, mut show, mut all) = (None, None, None, None, None, None, None);
    let mut column = "Русский".to_string();
    let (mut p, mut size, mut weight) = (Params::default(), None, None);
    while let Some(a) = args.next() {
        let mut v = || args.next().ok_or_else(|| format!("{} wants a value", a));
        match a.as_str() {
            "--rom" => rom = Some(PathBuf::from(v()?)),
            "--ttf" => ttf = Some(PathBuf::from(v()?)),
            "--table" => table = Some(PathBuf::from(v()?)),
            "--out" => out = Some(PathBuf::from(v()?)),
            "--base-font" => base_font = Some(PathBuf::from(v()?)),
            "--all" => all = Some(PathBuf::from(v()?)),
            "--column" => column = v()?,
            "--size" => size = Some(v()?.parse().map_err(|_| "--size wants a number")?),
            "--weight" => weight = Some(v()?.parse().map_err(|_| "--weight wants a number")?),
            "--show" => show = Some(v()?),
            "--pixel" => p.pixel = true,
            _ => return Err(format!("unknown argument {}", a)),
        }
    }
    // the pixel look wants the capitals nine pixels tall and a heavier stroke
    let (dsize, dweight) = if p.pixel { (13.0, 1.5) } else { (12.0, 1.0) };
    (p.size, p.weight) = (size.unwrap_or(dsize), weight.unwrap_or(dweight));
    let read = |p: &PathBuf| std::fs::read(p).map_err(|e| format!("{}: {}", p.display(), e));
    let rom_bytes = rom.as_ref().map(|r| read(r)).transpose()?;
    let from_rom = |path: &str| -> Result<Vec<u8>, String> {
        let rom = rom_bytes.as_ref().ok_or("--rom GAME.nds (or --base-font FILE.nftr) is needed")?;
        Ok(NitroFs::parse(rom)?.file(rom, path).ok_or_else(|| format!("{} is not in the ROM", path))?.to_vec())
    };
    let base_bytes = match &base_font {
        Some(f) => read(f)?,
        None => from_rom(DIALOGUE_FONT)?,
    };
    let base = Nftr::parse(&base_bytes)?;
    let ttf = read(&ttf.ok_or("--ttf is needed")?)?;
    let csv = String::from_utf8(read(&table.ok_or("--table is needed")?)?).map_err(|e| e.to_string())?;
    let rows = parse_csv(&csv);
    let col = rows.first().and_then(|h| h.iter().position(|c| *c == column)).ok_or_else(|| format!("the table has no column {}", column))?;
    let chars = fontgen::charset(rows.iter().skip(1).filter_map(|r| r.get(col)).map(|s| s.as_str()));
    if let Some(dir) = all {
        // every font of the game, the size following the height of its cell (the dialogue font is 14)
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {}", dir.display(), e))?;
        for path in FONTS {
            let base = Nftr::parse(&from_rom(path)?)?;
            let q = Params { size: p.size * base.cell_height as f32 / 14.0, ..p };
            let font = fontgen::build(&base, &ttf, &chars, &q)?;
            let out = dir.join(font_file_name(path));
            std::fs::write(&out, font.write()).map_err(|e| format!("{}: {}", out.display(), e))?;
            println!("{}: {}x{} cell, {} bit, size {:.1}", out.display(), base.cell_width, base.cell_height, base.depth, q.size);
        }
        return Ok(());
    }
    let font = fontgen::build(&base, &ttf, &chars, &p)?;
    if let Some(text) = show {
        print_line(&font, &text);
    }
    let out = out.ok_or("--out is needed")?;
    let bytes = font.write();
    std::fs::write(&out, &bytes).map_err(|e| format!("{}: {}", out.display(), e))?;
    println!("{}: {} glyphs for {} characters, {} bytes", out.display(), font.glyphs.len(), chars.len(), bytes.len());
    Ok(())
}

/// A line of text as the game would put it: each glyph at its pen position.
fn print_line(f: &Nftr, text: &str) {
    let (mut rows, mut pen) = (vec![String::new(); f.cell_height as usize], 0i32);
    let map: std::collections::HashMap<u16, u16> = f
        .maps
        .iter()
        .flat_map(|m| match &m.data {
            sunpetal_data::nftr::MapData::Scan(p) => p.clone(),
            _ => Vec::new(),
        })
        .collect();
    for c in text.chars() {
        let Some(&gi) = map.get(&(c as u32 as u16)) else { continue };
        let (bearing, ink, adv) = f.widths[gi as usize];
        let px = fontgen::unpack(&f.glyphs[gi as usize], f.depth, f.cell_width as usize * f.cell_height as usize);
        let left = pen + bearing as i32;
        for (y, row) in rows.iter_mut().enumerate() {
            while (row.chars().count() as i32) < left.max(0) {
                row.push(' ');
            }
            for x in 0..ink as usize {
                let v = px[y * f.cell_width as usize + x] as usize;
                let at = (left + x as i32).max(0) as usize;
                let ch = [' ', '.', ':', '-', '=', '+', '*', '#'][v.min(7)];
                if at < row.chars().count() {
                    *row = row.chars().enumerate().map(|(i, o)| if i == at && ch != ' ' { ch } else { o }).collect();
                } else {
                    row.push(ch);
                }
            }
        }
        pen += adv as i32 + 1;
    }
    for r in rows {
        println!("{}", r);
    }
}
