//! Puts a folder of 2D pictures (what `sunpetal-headless --dump-2d` wrote, drawn over or upscaled) into a data
//! file as its 2D part; the game takes them in place of the sprites and background layers they are named for.
//!
//!     sunpetal-pack-2d --dir PICTURES --out sunpetal.data [--merge OTHER.data]
//!
//! With `--merge` the other parts of that file are kept and its 2D part is replaced; without it the file
//! holds this part alone (add it to a game's data file with `--merge`).
use std::path::PathBuf;
use std::process::ExitCode;

use sunpetal_data::container::Container;

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
    let (mut dir, mut out, mut merge) = (None, None, None);
    while let Some(a) = args.next() {
        let mut v = || args.next().map(PathBuf::from).ok_or_else(|| format!("{} wants a value", a));
        match a.as_str() {
            "--dir" => dir = Some(v()?),
            "--out" => out = Some(v()?),
            "--merge" => merge = Some(v()?),
            _ => return Err(format!("unknown argument {}\nusage: sunpetal-pack-2d --dir PICTURES --out FILE [--merge FILE]", a)),
        }
    }
    let (dir, out) = (dir.ok_or("--dir is needed")?, out.ok_or("--out is needed")?);
    let mut c = match &merge {
        Some(m) => Container::read(m)?,
        None => Container::new(),
    };
    c.remove(sunpetal_data::pack2d::TAG);
    let part = sunpetal_data::pack2d::pack_dir(&dir)?;
    let n = sunpetal_data::pack2d::unpack(&part)?.len();
    c.add(sunpetal_data::pack2d::TAG, part);
    c.write(&out)?;
    println!("{}: {} pictures", out.display(), n);
    Ok(())
}
