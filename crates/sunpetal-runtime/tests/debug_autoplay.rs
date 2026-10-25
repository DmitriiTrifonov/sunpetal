//! `--autoplay`: A after each line, the lines logged in order with their ids. Needs the ROM and a save that has
//! the requests open (`SUNPETAL_ROM`, `SUNPETAL_SAVE`; skipped without either); the save is copied first.
#![cfg(all(has_generated, feature = "debug-tools"))]
use std::path::PathBuf;

use sunpetal_runtime::session::{Autoplay, Session, Stop};

#[test]
fn autoplay_logs_the_opening_lines_in_order() {
    let (Some(rom), Some(save)) = (std::env::var_os("SUNPETAL_ROM").map(PathBuf::from), std::env::var_os("SUNPETAL_SAVE").map(PathBuf::from)) else {
        eprintln!("skipped: SUNPETAL_ROM and SUNPETAL_SAVE are not both set");
        return;
    };
    let copy = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("debug_autoplay.sav");
    std::fs::copy(&save, &copy).expect("the save's copy");
    let mut s = Session::open(rom, Some(copy)).expect("open");
    s.goto("C18_01", None).expect("goto");
    let stop = s.autoplay(&Autoplay { max_lines: 5, delay: 10, out: None }).expect("autoplay");
    assert_eq!(stop, Stop::MaxLines);
    let ids: Vec<_> = s.events().iter().map(|e| e.id.clone().unwrap_or_default()).collect();
    assert_eq!(&ids[..5], ["01001", "01002", "01003", "01004", "01005"]);
    assert!(s.events().iter().all(|e| e.scene.as_deref() == Some("C18_01") && e.lines >= 1));
}
