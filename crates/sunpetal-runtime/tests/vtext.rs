//! The text of the line the dialogue window shows, read without the debug tracing (`nds::dialogue_text`), is the
//! line the debug log reports: the app draws it in the window's resolution (docs/PLAN.md, 9в.10). Needs the ROM and a
//! save that has the requests open (`SUNPETAL_ROM`, `SUNPETAL_SAVE`; skipped without either); the save is copied first.
#![cfg(all(has_generated, feature = "debug-tools"))]
use std::path::PathBuf;

use sunpetal_runtime::nds;
use sunpetal_runtime::session::Session;

#[test]
fn the_shown_text_is_the_logged_line() {
    let (Some(rom), Some(save)) = (std::env::var_os("SUNPETAL_ROM").map(PathBuf::from), std::env::var_os("SUNPETAL_SAVE").map(PathBuf::from)) else {
        eprintln!("skipped: SUNPETAL_ROM and SUNPETAL_SAVE are not both set");
        return;
    };
    let copy = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("vtext.sav");
    std::fs::copy(&save, &copy).expect("the save's copy");
    let mut s = Session::open(rom, Some(copy)).expect("open");
    s.goto("C01_01", None).expect("goto");
    let mut seen = 0;
    for _ in 0..6 {
        let Some(line) = s.advance(6000).expect("advance") else { break };
        let (ptr, text) = nds::dialogue_text().expect("the window shows a line");
        assert!(ptr != 0);
        assert_eq!(text.replace("\r\n", "\n"), line.text.replace("\r\n", "\n"), "line {:?}", line.id);
        seen += 1;
    }
    assert!(seen >= 5, "only {} lines", seen);
}
