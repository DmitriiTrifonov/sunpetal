//! The machine from the boot: the game's recompiled programs run headless for 300 VBlanks, the
//! SDK's threads and interrupts and the card's reads included, and the 2D engines draw the logos. Needs the ROM the
//! generated code was made from: `SUNPETAL_ROM=path cargo test` (skipped without).
#![cfg(has_generated)]
use std::path::PathBuf;

use sunpetal_runtime::nds::{self, Config};

#[test]
fn boots_and_runs() {
    let Some(rom) = std::env::var_os("SUNPETAL_ROM").map(PathBuf::from) else {
        eprintln!("skipped: SUNPETAL_ROM is not set");
        return;
    };
    let mut cfg = Config::new(rom);
    cfg.out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("boot");
    cfg.stop_vblanks = 300;
    nds::boot(cfg).expect("boot");
    let shot = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("boot/v300.png");
    loop {
        nds::set_draw(nds::vblanks() + 1 == 300);
        if !nds::run_frame() {
            break;
        }
        if nds::vblanks() == 300 {
            nds::screenshot(&shot).expect("screenshot");
        }
    }
    assert_eq!(nds::stopped(), Some("VBlank limit"));
    assert!(nds::vblanks() >= 300);
    // the publisher's logos are on both screens at frame 300: a blank picture is about 1 KB
    let size = std::fs::metadata(&shot).expect("the picture").len();
    assert!(size > 20_000, "the picture at VBlank 300 is blank ({} bytes)", size);
    nds::report();
}

/// START pressed through the logos and the movie reaches the game's save-data menu (3D character
/// on the upper screen, the keys and the title on the lower): a busy picture, where a blank or
/// glitched one is a few KB; and the sound unit plays.
#[test]
fn reaches_the_save_data_menu() {
    let Some(rom) = std::env::var_os("SUNPETAL_ROM").map(PathBuf::from) else {
        eprintln!("skipped: SUNPETAL_ROM is not set");
        return;
    };
    let mut cfg = Config::new(rom);
    cfg.out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("menu");
    cfg.stop_vblanks = 1500;
    cfg.input = (400..1400).step_by(100).map(|v| format!("{}:START", v)).collect::<Vec<_>>().join(",");
    nds::boot(cfg).expect("boot");
    let shot = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("menu/v1500.png");
    nds::set_sound_keep(true);
    let mut loud = 0usize;
    loop {
        nds::set_draw(nds::vblanks() + 1 == 1500);
        if !nds::run_frame() {
            break;
        }
        loud += nds::take_samples().iter().filter(|s| s.unsigned_abs() > 1000).count();
        if nds::vblanks() == 1500 {
            nds::screenshot(&shot).expect("screenshot");
        }
    }
    let size = std::fs::metadata(&shot).expect("the picture").len();
    assert!(size > 40_000, "the picture at VBlank 1500 is not the menu ({} bytes)", size);
    // the music has started by then (it is silent for the first twenty seconds)
    assert!(loud > 10_000, "no sound by VBlank 1500 ({} loud samples)", loud);
}
