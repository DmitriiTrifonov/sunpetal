//! 8е.7з: the background layers of the device (upper screen) and of the case menu (lower screen) are replaced by a
//! 2D pack. Needs the ROM and a save that has the requests open (`SUNPETAL_ROM`, `SUNPETAL_SAVE`; skipped
//! without either); the save is copied first. Without a pack the frame is as it was; a pack of one colour paints
//! the layers' pixels and changes nothing else (the digit, the lamp and the icons stay sprites).
#![cfg(all(has_generated, feature = "debug-tools"))]
use std::path::{Path, PathBuf};

use sunpetal_runtime::nds;
use sunpetal_runtime::session::{Autoplay, Session};

fn inputs() -> Option<(PathBuf, PathBuf)> {
    let rom = std::env::var_os("SUNPETAL_ROM").map(PathBuf::from);
    let save = std::env::var_os("SUNPETAL_SAVE").map(PathBuf::from);
    if rom.is_none() || save.is_none() {
        eprintln!("skipped: SUNPETAL_ROM and SUNPETAL_SAVE are not both set");
    }
    rom.zip(save)
}

fn solid(path: &Path, w: u32, h: u32, rgba: [u8; 4]) {
    let px: Vec<u8> = (0..w * h).flat_map(|_| rgba).collect();
    let f = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&px).unwrap();
}

/// One frame drawn: both screens as RGBA at the current scale.
fn frame(s: &mut Session) -> Vec<u8> {
    let png = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("bgswap_frame.png");
    s.shot(&png).expect("shot");
    nds::screens_rgba().1
}

#[test]
fn a_2d_pack_replaces_the_device_and_the_menu_backgrounds() {
    let Some((rom, save)) = inputs() else { return };
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("bgswap");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let copy = tmp.join("g.sav");
    std::fs::copy(&save, &copy).expect("the save's copy");
    let mut s = Session::open(rom, Some(copy)).expect("open");
    s.goto("C02_01", None).expect("goto");
    s.run(120).unwrap();
    // as `sunpetal-debug --goto C02_01 --autoplay` leaves the game
    s.autoplay(&Autoplay { max_lines: 200, delay: 120, out: None, screens: None }).unwrap();
    // the route of the screen-recon skill: the device, then the case menu below it
    s.skip(100).unwrap();
    s.advance(20).unwrap();
    s.press("START", 3).unwrap();
    s.advance(40).unwrap();
    s.tap(25, 43, 4).unwrap();
    s.advance(100).unwrap();
    s.press("RIGHT", 3).unwrap();
    s.advance(60).unwrap();

    let dump = tmp.join("dump");
    assert!(nds::dump_2d(&dump) > 0);
    let layers: Vec<String> = std::fs::read_dir(&dump).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("bg")).collect();
    // the device's picture (engine 0 layer 0) and the menu's (engine 1 layer 1)
    assert!(layers.iter().any(|n| n.starts_with("bg00_")) && layers.iter().any(|n| n.starts_with("bg11_")), "layers: {:?}", layers);

    nds::set_scale(4);
    let plain = frame(&mut s);
    let (w, h) = (1024usize, 768usize);
    assert_eq!(plain.len(), w * 2 * h * 4);

    // 1. one-colour pictures at twice the DS's size, shot at four times: the layers' pixels turn magenta
    let pack = tmp.join("pack");
    std::fs::create_dir_all(&pack).unwrap();
    for n in layers.iter().filter(|n| n.starts_with("bg00_") || n.starts_with("bg11_")) {
        solid(&pack.join(n), 512, 384, [255, 0, 255, 255]);
    }
    assert!(nds::set_pack_2d(Some(&pack)).unwrap() >= 2);
    let painted = frame(&mut s);
    assert!(nds::pack_2d_placed() >= 2, "the layers are placed");
    let (mut magenta, mut other) = (0usize, 0usize);
    for (a, b) in plain.chunks_exact(4).zip(painted.chunks_exact(4)) {
        if b[..3] == [255, 0, 255] {
            magenta += 1;
        } else if a != b {
            other += 1;
        }
    }
    let total = w * 2 * h;
    assert!(magenta > total / 3, "the layers cover most of both screens ({} of {})", magenta, total);
    // the frame between the two shots may move a sprite or two (a blinking corner); not more
    assert!(other < total / 100, "{} pixels changed to something else than the pack's colour", other);

    // 2. the sprites on top stay: the icons of the menu are not painted over (some pixels of the lower screen are not magenta)
    let lower_kept = painted[w * h * 4..].chunks_exact(4).filter(|p| p[..3] != [255, 0, 255]).count();
    assert!(lower_kept > 1000, "the menu's icons are sprites and stay ({} pixels)", lower_kept);

    // 3. without the pack the frame is as it was
    nds::set_pack_2d(None).unwrap();
    let again = frame(&mut s);
    let diff = plain.chunks_exact(4).zip(again.chunks_exact(4)).filter(|(a, b)| a != b).count();
    assert!(diff < total / 100, "{} pixels differ after the pack is dropped", diff);
}
