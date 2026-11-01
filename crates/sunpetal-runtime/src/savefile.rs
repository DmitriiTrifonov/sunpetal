//! The game's save file (`.sav`, the raw bytes of the backup memory as melonDS and DeSmuME keep
//! them): the check of a file to import, the backup of the one it replaces, and the write that
//! leaves either the old file or the new one whole.
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The sizes a DS backup chip has: EEPROM 512 B, 8 KiB, 64 KiB, 128 KiB; flash 256 KiB to 1 MiB and 8 MiB.
pub const SIZES: [usize; 8] = [512, 8192, 65536, 131072, 262144, 524288, 1048576, 8388608];

/// How many backups (`NAME.sav.bak-STAMP`) stay beside the save: the newest.
pub const KEEP_BACKUPS: usize = 5;

/// Whether `bytes` can be a save: a chip's size, and something written to it (a file of 0xFF or of
/// 0 only is an empty chip, and replacing a save by it would erase the progress).
pub fn check(bytes: &[u8]) -> Result<(), String> {
    if !SIZES.contains(&bytes.len()) {
        let sizes: Vec<String> = SIZES.iter().map(|n| n.to_string()).collect();
        return Err(format!("not a save file: {} bytes, a DS save is one of {}", bytes.len(), sizes.join(", ")));
    }
    if bytes.iter().all(|b| *b == 0xFF) || bytes.iter().all(|b| *b == 0) {
        return Err("the file is empty: no byte of it was ever written by a game".into());
    }
    Ok(())
}

/// The save's file for a run: the one named, else `out/<name of the ROM, or of the data file>.sav`.
pub fn default_path(named: Option<&Path>, out: &Path, source: &Path) -> PathBuf {
    match named {
        Some(p) => p.to_path_buf(),
        None => out.join(format!("{}.sav", source.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())),
    }
}

/// `bytes` into `path` whole, then moved into place: a kill in the middle leaves the old file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(d) = path.parent() {
        let _ = fs::create_dir_all(d);
    }
    let tmp = path.with_extension("sav.tmp");
    fs::write(&tmp, bytes).map_err(|e| format!("cannot write {}: {}", tmp.display(), e))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("cannot move {} into place: {}", tmp.display(), e)
    })
}

/// `YYYYMMDD-HHMMSS` (UTC) of a moment in seconds since 1970 (days to a date as in Howard Hinnant's `civil_from_days`).
fn stamp(secs: u64) -> String {
    let (days, rest) = ((secs / 86400) as i64, secs % 86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", y, m, d, rest / 3600, rest % 3600 / 60, rest % 60)
}

/// The backups beside `path`, oldest first (their names sort by the stamp).
fn backups(path: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else { return Vec::new() };
    let prefix = format!("{}.bak-", name.to_string_lossy());
    let dir = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&prefix))).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// A copy of the save at `path` (if there is one) as `NAME.bak-STAMP`, the oldest copies past `keep` removed.
/// The copy's name when one was made.
pub fn backup(path: &Path, keep: usize) -> Result<Option<PathBuf>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let named = |suffix: String| {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        PathBuf::from(name)
    };
    // a counter after the stamp: two imports in one second, or a clock set back, still give a name
    // that sorts after every copy there is (so the pruning below removes the oldest, never the new one)
    let last = backups(path).pop();
    let mut n = 0;
    let to = loop {
        let c = named(format!(".bak-{}-{:03}", stamp(now), n));
        if !c.exists() && last.as_ref().is_none_or(|l| c > *l) {
            break c;
        }
        n += 1;
    };
    fs::copy(path, &to).map_err(|e| format!("cannot keep a copy of {} as {}: {}", path.display(), to.display(), e))?;
    let all = backups(path);
    for old in all.iter().take(all.len().saturating_sub(keep.max(1))) {
        let _ = fs::remove_file(old);
    }
    Ok(Some(to))
}

/// Puts the save in `src` in place of the one at `dest`: checked first, the old one kept as a
/// backup, then written whole. The words for the player.
pub fn import(src: &Path, dest: &Path) -> Result<String, String> {
    let bytes = fs::read(src).map_err(|e| format!("cannot read {}: {}", src.display(), e))?;
    check(&bytes).map_err(|e| format!("{}: {}", src.display(), e))?;
    let before = fs::metadata(dest).map(|m| m.len() as usize).ok();
    let kept = backup(dest, KEEP_BACKUPS)?;
    write_atomic(dest, &bytes)?;
    let mut o = format!("save imported from {} ({} bytes)", src.display(), bytes.len());
    if let Some(b) = kept {
        o += &format!("; the old one is kept as {}", b.display());
    }
    if let Some(n) = before.filter(|n| *n != bytes.len()) {
        o += &format!("; its size differs from the old one's {} bytes", n);
    }
    Ok(o)
}

/// The file that says a game is running on a save: `NAME.sav.lock` holds the process number.
pub fn lock_path(save: &Path) -> PathBuf {
    let mut n = save.as_os_str().to_os_string();
    n.push(".lock");
    PathBuf::from(n)
}

/// Whether a process with this number is alive (`kill -0`; where that is missing, it is taken to be).
fn alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    #[cfg(unix)]
    {
        return std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(true);
    }
    #[allow(unreachable_code)]
    true
}

/// The process that has a game running on `save`, if the lock file names a live one (a lock left by
/// a process that is gone does not count).
pub fn running(save: &Path) -> Option<u32> {
    let pid: u32 = fs::read_to_string(lock_path(save)).ok()?.trim().parse().ok()?;
    alive(pid).then_some(pid)
}

/// A game's claim on a save: the lock file exists while this value does (and is removed with it).
#[derive(Debug)]
pub struct Lock {
    path: PathBuf,
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Takes the lock on `save` for this process. Another live process holding it is an error (two games on
/// one save write over each other); a lock left by a dead one is taken over.
pub fn lock(save: &Path) -> Result<Lock, String> {
    use std::io::Write;
    let path = lock_path(save);
    if let Some(pid) = running(save) {
        if pid != std::process::id() {
            return Err(format!("a game (process {}) is already running on {}", pid, save.display()));
        }
    }
    if let Some(d) = path.parent() {
        let _ = fs::create_dir_all(d);
    }
    let mut f = fs::File::create(&path).map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
    let _ = write!(f, "{}", std::process::id());
    Ok(Lock { path })
}

/// Writes a copy of the save to `to` (the raw bytes, as other emulators read them).
pub fn export(bytes: &[u8], to: &Path) -> Result<String, String> {
    check(bytes).map_err(|e| format!("nothing to export: {}", e))?;
    write_atomic(to, bytes)?;
    Ok(format!("save exported to {} ({} bytes)", to.display(), bytes.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sunpetal-savefile-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn save(seed: u8) -> Vec<u8> {
        let mut v = vec![0xFF; 65536];
        for (i, b) in v.iter_mut().take(300).enumerate() {
            *b = (i as u8).wrapping_mul(7).wrapping_add(seed);
        }
        v
    }

    #[test]
    fn a_save_is_a_chips_size_with_something_written() {
        assert!(check(&save(1)).is_ok());
        assert!(check(&vec![0u8; 512]).is_err_and(|e| e.contains("empty")));
        assert!(check(&vec![0xFFu8; 8192]).is_err_and(|e| e.contains("empty")));
        let e = check(&vec![1u8; 1234]).unwrap_err();
        assert!(e.contains("1234 bytes") && e.contains("65536"), "{}", e);
        assert!(check(&[]).is_err());
    }

    #[test]
    fn an_export_and_an_import_give_back_the_same_bytes() {
        let d = dir("round");
        let a = save(3);
        let out = d.join("out.sav");
        export(&a, &out).unwrap();
        assert_eq!(fs::read(&out).unwrap(), a);
        let dest = d.join("game.sav");
        fs::write(&dest, save(9)).unwrap();
        let said = import(&out, &dest).unwrap();
        assert!(said.contains("kept as"), "{}", said);
        assert_eq!(fs::read(&dest).unwrap(), a);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_wrong_file_is_refused_and_the_save_stays() {
        let d = dir("wrong");
        let dest = d.join("game.sav");
        fs::write(&dest, save(5)).unwrap();
        let bad = d.join("bad.sav");
        fs::write(&bad, vec![7u8; 1000]).unwrap();
        let e = import(&bad, &dest).unwrap_err();
        assert!(e.contains("1000 bytes"), "{}", e);
        assert!(import(&d.join("missing.sav"), &dest).is_err());
        assert_eq!(fs::read(&dest).unwrap(), save(5));
        assert!(backups(&dest).is_empty(), "a refused import keeps no copy");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_old_save_is_kept_and_only_the_newest_copies_stay() {
        let d = dir("backup");
        let dest = d.join("game.sav");
        let new = d.join("new.sav");
        fs::write(&new, save(1)).unwrap();
        for i in 0..8u8 {
            fs::write(&dest, save(10 + i)).unwrap();
            import(&new, &dest).unwrap();
        }
        let kept = backups(&dest);
        assert_eq!(kept.len(), KEEP_BACKUPS);
        // the newest copy is the save the last import replaced
        assert_eq!(fs::read(kept.last().unwrap()).unwrap(), save(17));
        assert_eq!(fs::read(&dest).unwrap(), save(1));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_write_leaves_no_half_file_and_no_temporary_one() {
        let d = dir("atomic");
        let dest = d.join("game.sav");
        write_atomic(&dest, &save(4)).unwrap();
        write_atomic(&dest, &save(6)).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), save(6));
        assert!(!dest.with_extension("sav.tmp").exists());
        // the temporary name is taken by a directory: the write fails and the save is as it was
        fs::create_dir(dest.with_extension("sav.tmp")).unwrap();
        assert!(write_atomic(&dest, &save(8)).is_err());
        assert_eq!(fs::read(&dest).unwrap(), save(6));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_stamp_is_a_utc_date_and_time() {
        assert_eq!(stamp(0), "19700101-000000");
        // 2025-11-01 12:26:40 UTC
        assert_eq!(stamp(1_762_000_000), "20251101-122640");
    }

    #[test]
    fn the_default_path_is_the_named_one_or_beside_the_run() {
        assert_eq!(default_path(Some(Path::new("a/b.sav")), Path::new("out"), Path::new("x.nds")), PathBuf::from("a/b.sav"));
        assert_eq!(default_path(None, Path::new("out"), Path::new("dir/game.nds")), PathBuf::from("out/game.sav"));
    }
    #[test]
    fn a_running_game_holds_the_lock_and_a_dead_ones_lock_is_taken_over() {
        let d = dir("lock");
        let save = d.join("g.sav");
        assert_eq!(running(&save), None);
        {
            let _l = lock(&save).unwrap();
            assert!(lock_path(&save).exists());
            assert_eq!(running(&save), Some(std::process::id()));
        }
        assert!(!lock_path(&save).exists(), "the lock goes with the value");
        // a lock of a process that is gone (the number of a finished child)
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let dead = child.id();
        child.wait().unwrap();
        fs::write(lock_path(&save), dead.to_string()).unwrap();
        assert_eq!(running(&save), None, "a dead process does not hold it");
        let l = lock(&save).unwrap();
        assert_eq!(running(&save), Some(std::process::id()));
        drop(l);
        // a live other process refuses
        let mut sleeper = std::process::Command::new("sleep").arg("5").spawn().unwrap();
        fs::write(lock_path(&save), sleeper.id().to_string()).unwrap();
        let e = lock(&save).unwrap_err();
        assert!(e.contains("already running"), "{}", e);
        sleeper.kill().unwrap();
        sleeper.wait().unwrap();
        fs::remove_dir_all(&d).unwrap();
    }
}
