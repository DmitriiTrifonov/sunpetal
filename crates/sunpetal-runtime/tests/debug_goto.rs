//! `--goto`: the menu of requests leads to the start of a chapter. Needs the ROM and a save that has the
//! requests open (`SUNPETAL_ROM`, `SUNPETAL_SAVE`; skipped without either); the save is copied first.
#![cfg(all(has_generated, feature = "debug-tools"))]
use std::path::PathBuf;

use sunpetal_runtime::session::Session;

fn inputs() -> Option<(PathBuf, PathBuf)> {
    let rom = std::env::var_os("SUNPETAL_ROM").map(PathBuf::from);
    let save = std::env::var_os("SUNPETAL_SAVE").map(PathBuf::from);
    if rom.is_none() || save.is_none() {
        eprintln!("skipped: SUNPETAL_ROM and SUNPETAL_SAVE are not both set");
    }
    rom.zip(save)
}

#[test]
fn goto_a_chapter_reads_its_first_scene() {
    let Some((rom, save)) = inputs() else { return };
    let copy = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("debug_goto.sav");
    std::fs::copy(&save, &copy).expect("the save's copy");
    let mut s = Session::open(rom, Some(copy)).expect("open");
    s.goto("C04_01", None).expect("goto");
    assert_eq!(s.advance(6000).expect("advance").and_then(|e| e.scene).as_deref(), Some("C04_01"));
}
