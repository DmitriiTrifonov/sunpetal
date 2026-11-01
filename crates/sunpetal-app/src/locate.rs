//! Finding the game's files when the command line does not name them: the data file
//! (`sunpetal.data`) and, as the way around it, a ROM of the supported dump (docs/PLAN.md, stage 9b).
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The data file's name where it is looked for.
pub const DATA_NAME: &str = "sunpetal.data";
/// The game code of the supported dump (the European one).
const GAME_CODE: &[u8; 4] = b"YHNP";

/// Where files are looked for: the current folder, the program's, the configuration folder.
pub fn search_dirs() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from(".")];
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        v.push(d);
    }
    v.push(crate::config::dir());
    v
}

/// The first `sunpetal.data` in `dirs`.
pub fn find_data(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter().map(|d| d.join(DATA_NAME)).find(|p| p.is_file())
}

/// Whether the file is a ROM of the supported dump: the message if not.
pub fn check_rom(path: &Path) -> Result<(), String> {
    let mut head = [0u8; 0x10];
    let mut f = fs::File::open(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    f.read_exact(&mut head).map_err(|_| format!("{}: too short to be a DS ROM", path.display()))?;
    if &head[0xC..] == GAME_CODE {
        Ok(())
    } else {
        Err(format!(
            "{}: not the supported ROM (game code {:?}, expected {}: the European release of the game)",
            path.display(),
            String::from_utf8_lossy(&head[0xC..]),
            String::from_utf8_lossy(GAME_CODE)
        ))
    }
}

/// Whether a data file holds as `META` records it (the check `sunpetal-check` makes): the message
/// if it is damaged, cut short, or not a data file. A file made before `META` passes unchecked.
pub fn check_data(path: &Path) -> Result<(), String> {
    sunpetal_data::container::verify_file(path).map(|_| ())
}

/// Whether a data file was made from this ROM (its `META`'s game code and CRC-32 against the ROM
/// file's): the message if not. A data file without `META` passes unchecked.
pub fn check_pair(data: &Path, rom: &Path) -> Result<(), String> {
    let c = sunpetal_data::container::Container::read_only(data, |t| t == b"META")?;
    let Some(meta) = c.meta()? else { return Ok(()) };
    let image = fs::read(rom).map_err(|e| format!("{}: {}", rom.display(), e))?;
    meta.check_image(&image).map_err(|e| format!("{} with {}: {}", data.display(), rom.display(), e))
}

/// Tells the player why the game does not start as asked: the terminal, and on a desktop a message
/// box (the window is not up yet, and a double-clicked program has no terminal to read).
pub fn tell(message: &str) {
    eprintln!("{}", message);
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    rfd::MessageDialog::new().set_title("Sunpetal").set_description(message).set_level(rfd::MessageLevel::Error).show();
}

/// What the player picked in the file dialog.
#[cfg_attr(any(target_os = "android", target_os = "ios"), allow(dead_code))]
#[derive(Debug, PartialEq)]
pub enum Picked {
    /// A ROM of the supported dump: goes to `[game] rom`.
    Rom,
    /// A data file that holds the game: goes to the texture setting, where the data file lives.
    Data,
}

/// Tells what a picked file is, or why it is no game: the message for the player.
#[cfg_attr(any(target_os = "android", target_os = "ios"), allow(dead_code))]
pub fn classify(path: &Path) -> Result<Picked, String> {
    if !path.is_file() {
        return Err(format!("{}: not a file", path.display()));
    }
    if sunpetal_data::dsrom::has_rom(path) {
        check_data(path)?;
        return Ok(Picked::Data);
    }
    check_rom(path).map(|_| Picked::Rom).map_err(|e| {
        format!("{}\n\nPick the European release's .nds, or a sunpetal.data that holds the game (made by sunpetal-build).", e)
    })
}

/// The language of a translation in the data file that is the system's (`ru` for `ru_RU.UTF-8`
/// or `ru-RU`); none if the file has none such or cannot be read.
#[cfg_attr(target_os = "ios", allow(dead_code))]
pub fn language_for(path: &Path, locale: &str) -> Option<String> {
    let want = locale.split(['-', '_', '.']).next()?.to_ascii_lowercase();
    let c = sunpetal_data::container::Container::read_only(path, |t| t == b"TRAN").ok()?;
    let found = c.all(b"TRAN").filter_map(|s| sunpetal_data::translate::Translation::from_bytes(&s.data).ok()).map(|t| t.lang).find(|l| l.eq_ignore_ascii_case(&want));
    found
}

/// The one supported ROM among the `.nds` files of `dirs`; none if there is none.
pub fn find_rom(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter().find_map(|d| {
        let mut found: Vec<PathBuf> = fs::read_dir(d)
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("nds")) && check_rom(p).is_ok())
            .collect();
        found.sort();
        found.into_iter().next()
    })
}

/// The message for a start with no ROM to play.
pub fn no_game(dirs: &[PathBuf]) -> String {
    let places: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
    format!(
        "no ROM to play: give one (sunpetal-app ROM.nds, or `rom` under [game] in {}), or put a .nds of the European release, or a {} that holds the game (sunpetal-pack-rom), in one of: {}",
        crate::config::dir().join("config.toml").display(),
        DATA_NAME,
        places.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sunpetal-locate-{}-{}", name, std::process::id()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn rom(code: &[u8; 4]) -> Vec<u8> {
        let mut v = vec![0u8; 0x200];
        v[0xC..0x10].copy_from_slice(code);
        v
    }

    #[test]
    fn it_takes_only_the_supported_rom() {
        let d = dir("rom");
        fs::write(d.join("other.nds"), rom(b"ABCD")).unwrap();
        fs::write(d.join("notes.txt"), b"x").unwrap();
        assert_eq!(find_rom(&[d.clone()]), None);
        fs::write(d.join("game.nds"), rom(b"YHNP")).unwrap();
        assert_eq!(find_rom(&[d.clone()]), Some(d.join("game.nds")));
        assert!(check_rom(&d.join("other.nds")).unwrap_err().contains("ABCD"));
        assert!(check_rom(&d.join("notes.txt")).unwrap_err().contains("too short"));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn it_refuses_a_damaged_data_file_with_the_reason() {
        let d = dir("data");
        let mut c = sunpetal_data::container::Container::new();
        c.add(b"TRAN", b"part".to_vec());
        c.set_rom(*b"YHNP", 7);
        let f = d.join(DATA_NAME);
        c.write(&f).unwrap();
        assert!(check_data(&f).is_ok());
        let mut b = fs::read(&f).unwrap();
        let last = b.len() - 1;
        b[last] ^= 1;
        fs::write(&f, b).unwrap();
        assert!(check_data(&f).unwrap_err().contains("damaged"));
        fs::write(&f, b"not a data file at all").unwrap();
        assert!(check_data(&f).is_err());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn it_tells_a_rom_from_a_data_file_and_from_anything_else() {
        let d = dir("classify");
        fs::write(d.join("game.nds"), rom(b"YHNP")).unwrap();
        fs::write(d.join("other.nds"), rom(b"ABCD")).unwrap();
        fs::write(d.join("notes.txt"), b"hello").unwrap();
        assert_eq!(classify(&d.join("game.nds")), Ok(Picked::Rom));
        assert!(classify(&d.join("other.nds")).unwrap_err().contains("ABCD"));
        assert!(classify(&d.join("notes.txt")).is_err());
        assert!(classify(&d.join("missing")).unwrap_err().contains("not a file"));
        // a data file with the ROM in it
        let mut image = rom(b"YHNP");
        image.resize(0x400, 0);
        let mut c = sunpetal_data::container::Container::new();
        c.add(sunpetal_data::dsrom::TAG, sunpetal_data::dsrom::pack(&image).unwrap());
        c.set_rom(*b"YHNP", 1);
        c.write(&d.join("sunpetal.data")).unwrap();
        assert_eq!(classify(&d.join("sunpetal.data")), Ok(Picked::Data));
        // the same file damaged is refused with the reason
        let mut b = fs::read(d.join("sunpetal.data")).unwrap();
        let last = b.len() - 1;
        b[last] ^= 1;
        fs::write(d.join("bad.data"), b).unwrap();
        assert!(classify(&d.join("bad.data")).unwrap_err().contains("damaged"));
        fs::remove_dir_all(&d).unwrap();
    }

    /// A data file beside a ROM it was not made from is told so; the one it was made from, and a
    /// file with no `META`, pass.
    #[test]
    fn a_data_file_is_checked_against_the_rom_beside_it() {
        let d = dir("pair");
        let mut image = rom(b"YHNP");
        image.resize(0x400, 7);
        fs::write(d.join("a.nds"), &image).unwrap();
        let mut other = image.clone();
        other[0x300] ^= 1;
        fs::write(d.join("b.nds"), &other).unwrap();
        let mut c = sunpetal_data::container::Container::new();
        c.add(b"TEXS", vec![0, 0, 0, 0]);
        c.set_rom(*b"YHNP", sunpetal_data::delta::crc32(&image));
        c.write(&d.join("a.data")).unwrap();
        assert_eq!(check_pair(&d.join("a.data"), &d.join("a.nds")), Ok(()));
        let e = check_pair(&d.join("a.data"), &d.join("b.nds")).unwrap_err();
        assert!(e.contains("another ROM") && e.contains("b.nds"), "{}", e);
        let mut old = sunpetal_data::container::Container::new();
        old.add(b"TEXS", vec![0, 0, 0, 0]);
        old.write(&d.join("old.data")).unwrap();
        assert_eq!(check_pair(&d.join("old.data"), &d.join("b.nds")), Ok(()));
        assert!(check_pair(&d.join("a.data"), &d.join("missing.nds")).is_err());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn it_picks_the_translation_that_is_the_systems_language() {
        let d = dir("lang");
        let mut c = sunpetal_data::container::Container::new();
        let tr = sunpetal_data::translate::Translation { lang: "ru".into(), base: "ue".into(), rom_crc: 1, ..Default::default() };
        c.add(b"TRAN", tr.to_bytes().unwrap());
        c.set_rom(*b"YHNP", 1);
        let f = d.join("ru.data");
        c.write(&f).unwrap();
        assert_eq!(language_for(&f, "ru_RU.UTF-8"), Some("ru".into()));
        assert_eq!(language_for(&f, "ru-RU"), Some("ru".into()));
        assert_eq!(language_for(&f, "RU"), Some("ru".into()));
        assert_eq!(language_for(&f, "en-US"), None);
        assert_eq!(language_for(&f, ""), None);
        assert_eq!(language_for(&d.join("missing"), "ru"), None);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn it_finds_the_data_file_in_the_first_folder_that_has_it() {
        let (a, b) = (dir("a"), dir("b"));
        assert_eq!(find_data(&[a.clone(), b.clone()]), None);
        fs::write(b.join(DATA_NAME), b"x").unwrap();
        assert_eq!(find_data(&[a.clone(), b.clone()]), Some(b.join(DATA_NAME)));
        fs::remove_dir_all(&a).unwrap();
        fs::remove_dir_all(&b).unwrap();
    }
}
