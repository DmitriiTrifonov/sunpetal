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

/// The line counts as drawn at the frame the game draws its first letter (`020338E0`), not when the pointer changes.
#[test]
fn a_line_is_drawn_when_the_game_draws_its_first_letter() {
    use std::cell::RefCell;
    use std::rc::Rc;
    use sunpetal_runtime::nds::debug;
    let (Some(rom), Some(save)) = (std::env::var_os("SUNPETAL_ROM").map(PathBuf::from), std::env::var_os("SUNPETAL_SAVE").map(PathBuf::from)) else {
        eprintln!("skipped: SUNPETAL_ROM and SUNPETAL_SAVE are not both set");
        return;
    };
    let copy = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("vtext2.sav");
    std::fs::copy(&save, &copy).expect("the save's copy");
    let mut s = Session::open(rom, Some(copy)).expect("open");
    s.goto("C01_01", None).expect("goto");
    // the frames at which the game drew a letter
    let glyph_frames: Rc<RefCell<Vec<u64>>> = Rc::default();
    let g = glyph_frames.clone();
    debug::watch(9, 0x0203_38E0, move |_| g.borrow_mut().push(nds::vblanks()));
    let (mut ptr, mut changed, mut lines) = (0u32, 0u64, Vec::new());
    for _ in 0..1500 {
        // A at a steady pace to move the lines on
        if nds::vblanks() % 90 == 0 {
            s.press("A", 3).unwrap();
        } else {
            s.run(1).unwrap();
        }
        let now = nds::vblanks();
        let Some((p, _)) = nds::dialogue_text() else { continue };
        if p != ptr {
            (ptr, changed) = (p, now);
        }
        if nds::dialogue().is_some_and(|d| d.drawn) && lines.last().map_or(true, |l: &(u32, u64, u64)| l.0 != p) {
            lines.push((p, changed, now));
        }
    }
    assert!(lines.len() >= 3, "only {} lines were drawn", lines.len());
    let first_glyph_after = |t: u64| glyph_frames.borrow().iter().copied().find(|f| *f >= t);
    for (p, changed, drawn) in &lines {
        let glyph = first_glyph_after(*changed).expect("the game drew the line");
        eprintln!("line {:08X}: pointer at {}, first letter at {}, drawn at {}", p, changed, glyph, drawn);
        assert!(*drawn >= glyph && *drawn <= glyph + 3, "drawn at {} but the first letter at {}", drawn, glyph);
        assert!(*drawn > *changed + 5, "drawn at once, before the game draws");
    }
}

/// One run to a fully drawn line, and its next frame drawn with or without the raster text hidden.
fn frame_after_a_drawn_line(rom: PathBuf, save: PathBuf, hide: bool) -> (usize, Vec<u8>) {
    let copy = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("vtext3{}.sav", hide as u8));
    std::fs::copy(&save, &copy).expect("the save's copy");
    let mut s = Session::open(rom, Some(copy)).expect("open");
    s.goto("C01_01", None).expect("goto");
    // `dialogue()` is polled each frame, as the app does; A moves the lines on until one is drawn
    let mut drawn = false;
    for i in 0..3000 {
        if i % 90 == 0 {
            s.press("A", 3).unwrap();
        } else {
            s.run(1).unwrap();
        }
        if nds::dialogue().is_some_and(|d| d.drawn) {
            drawn = true;
            break;
        }
    }
    assert!(drawn, "no line was drawn");
    // the whole line is drawn, nothing moves any more
    for _ in 0..90 {
        s.run(1).unwrap();
        nds::dialogue();
    }
    // the session draws only on request
    nds::hide_dialogue_raster(hide);
    nds::set_draw(true);
    s.run(1).unwrap();
    let out = nds::screens_rgba();
    nds::set_draw(false);
    nds::hide_dialogue_raster(false);
    out
}

/// With the raster text hidden the frame differs from the plain one only in the dialogue band (upper screen, lines 144..191).
#[test]
fn hiding_the_raster_text_changes_only_the_dialogue_band() {
    let (Some(rom), Some(save)) = (std::env::var_os("SUNPETAL_ROM").map(PathBuf::from), std::env::var_os("SUNPETAL_SAVE").map(PathBuf::from)) else {
        eprintln!("skipped: SUNPETAL_ROM and SUNPETAL_SAVE are not both set");
        return;
    };
    // the emulator's state belongs to its thread: each run has a thread of its own
    let run = |hide: bool| {
        let (rom, save) = (rom.clone(), save.clone());
        std::thread::spawn(move || frame_after_a_drawn_line(rom, save, hide)).join().unwrap()
    };
    let (scale, plain) = run(false);
    let (_, hidden) = run(true);
    assert_eq!(plain.len(), hidden.len());
    let row = 256 * scale * 4;
    let (mut inside, mut outside) = (0usize, 0usize);
    for (i, (a, b)) in plain.chunks_exact(4).zip(hidden.chunks_exact(4)).enumerate() {
        if a != b {
            let y = i * 4 / row;
            if (144 * scale..192 * scale).contains(&y) { inside += 1 } else { outside += 1 }
        }
    }
    eprintln!("changed: {} in the band, {} elsewhere", inside, outside);
    assert!(inside > 100, "the letters are still there");
    assert_eq!(outside, 0, "something other than the band changed");
}
