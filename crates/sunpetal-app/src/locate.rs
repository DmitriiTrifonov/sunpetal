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
    fn it_finds_the_data_file_in_the_first_folder_that_has_it() {
        let (a, b) = (dir("a"), dir("b"));
        assert_eq!(find_data(&[a.clone(), b.clone()]), None);
        fs::write(b.join(DATA_NAME), b"x").unwrap();
        assert_eq!(find_data(&[a.clone(), b.clone()]), Some(b.join(DATA_NAME)));
        fs::remove_dir_all(&a).unwrap();
        fs::remove_dir_all(&b).unwrap();
    }
}
