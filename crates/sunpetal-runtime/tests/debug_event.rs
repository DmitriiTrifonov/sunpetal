//! `--goto` to a scene in free walk: the doors from map to map, the story step, the event's place. Needs the
//! ROM (`SUNPETAL_ROM`) and a save that has the requests open (`SUNPETAL_SAVE`); skipped without either.
#![cfg(all(has_generated, feature = "debug-tools"))]
use std::path::PathBuf;

use sunpetal_runtime::session::{Autoplay, Session, Stop};

#[test]
fn goto_a_scene_in_free_walk_plays_its_lines() {
    let (Some(rom), Some(save)) = (std::env::var_os("SUNPETAL_ROM").map(PathBuf::from), std::env::var_os("SUNPETAL_SAVE").map(PathBuf::from)) else {
        eprintln!("skipped: SUNPETAL_ROM and SUNPETAL_SAVE are not both set");
        return;
    };
    let copy = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("debug_event.sav");
    std::fs::copy(&save, &copy).expect("the save's copy");
    let mut s = Session::open(rom, Some(copy)).expect("open");
    s.goto("C04_16", None).expect("goto");
    let stop = s.autoplay(&Autoplay { max_lines: 3, delay: 0, out: None }).expect("autoplay");
    assert_eq!(stop, Stop::MaxLines);
    let ids: Vec<_> = s.events().iter().rev().take(3).map(|e| e.id.clone().unwrap_or_default()).collect();
    assert_eq!(ids, ["16003", "16002", "16001"]);
}
