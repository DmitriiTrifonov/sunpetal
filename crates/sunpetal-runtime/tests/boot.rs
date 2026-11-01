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

/// One run of the game to `frames` VBlanks (START pressed through the logos), with only the last
/// frame drawn, its picture saved; in a thread of its own, since the runtime keeps its state per
/// thread. Returns the picture's bytes.
fn picture_at(cfg: Config, frames: u64, shot: PathBuf) -> Vec<u8> {
    picture_with(cfg, frames, shot, nds::Renderer::Software, &[])
}

/// `picture_at` with the renderer set at the start and switched at the VBlanks in `switches` (to the other one each time).
fn picture_with(cfg: Config, frames: u64, shot: PathBuf, first: nds::Renderer, switches: &[u64]) -> Vec<u8> {
    picture_failing(cfg, frames, shot, first, switches, false)
}

fn picture_failing(cfg: Config, frames: u64, shot: PathBuf, first: nds::Renderer, switches: &[u64], fail: bool) -> Vec<u8> {
    let switches = switches.to_vec();
    let shot2 = shot.clone();
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            let mut cfg = cfg;
            cfg.stop_vblanks = frames;
            cfg.input = (400..frames).step_by(100).map(|v| format!("{}:START", v)).collect::<Vec<_>>().join(",");
            nds::boot(cfg).expect("boot");
            let mut renderer = first;
            nds::set_renderer(renderer);
            #[cfg(feature = "gpu")]
            nds::debug_fail_gpu_frames(fail);
            let _ = fail;
            loop {
                if switches.contains(&nds::vblanks()) {
                    renderer = if renderer == nds::Renderer::Gpu { nds::Renderer::Software } else { nds::Renderer::Gpu };
                    nds::set_renderer(renderer);
                }
                nds::set_draw(nds::vblanks() + 1 == frames);
                if !nds::run_frame() {
                    break;
                }
                if nds::vblanks() == frames {
                    nds::screenshot(&shot2).expect("screenshot");
                }
            }
        })
        .unwrap()
        .join()
        .expect("the run");
    std::fs::read(&shot).expect("the picture")
}

/// The texture dump and the texture pack agree: a pack of what the game dumped draws the same
/// picture as no pack, so the keys, the names and the replacement path all hold together.
#[test]
fn a_pack_made_of_the_dump_changes_nothing() {
    let Some(rom) = std::env::var_os("SUNPETAL_ROM").map(PathBuf::from) else {
        eprintln!("skipped: SUNPETAL_ROM is not set");
        return;
    };
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("textures");
    let _ = std::fs::remove_dir_all(&tmp);
    let dump = tmp.join("dump");
    let mut cfg = Config::new(&rom);
    cfg.out = tmp.join("a");
    cfg.dump_textures = Some(dump.clone());
    let plain = picture_at(cfg, 2400, tmp.join("a.png"));
    std::thread::sleep(std::time::Duration::from_millis(500)); // the dump's writer threads
    let n = std::fs::read_dir(&dump).map(|d| d.flatten().filter(|e| e.path().extension().map_or(false, |x| x == "png")).count()).unwrap_or(0);
    assert!(n >= 5, "the 3D scene at VBlank 2400 uses textures ({} dumped)", n);
    let mut cfg = Config::new(&rom);
    cfg.out = tmp.join("b");
    cfg.texture_pack = Some(dump);
    let packed = picture_at(cfg, 2400, tmp.join("b.png"));
    assert!(plain == packed, "a pack of the dump must draw the same picture");
}

/// The 2D dump (этап 8е.2): two runs of the same frames write the same pictures under the same names and the
/// same `index.csv`; both screens give background layers, and the sprites carry their size in the name.
#[test]
fn the_2d_dump_is_the_same_on_every_run() {
    let Some(rom) = std::env::var_os("SUNPETAL_ROM").map(PathBuf::from) else {
        eprintln!("skipped: SUNPETAL_ROM is not set");
        return;
    };
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("dump2d");
    let _ = std::fs::remove_dir_all(&tmp);
    let one = |name: &str| -> (Vec<String>, String) {
        let dir = tmp.join(name);
        let (cfg, d2) = (Config::new(&rom), dir.clone());
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(move || {
                let mut cfg = cfg;
                cfg.stop_vblanks = 1600;
                cfg.input = (400..1600).step_by(100).map(|v| format!("{}:START", v)).collect::<Vec<_>>().join(",");
                nds::boot(cfg).expect("boot");
                while nds::run_frame() {
                    if nds::vblanks() % 100 == 0 {
                        nds::dump_2d(&d2);
                    }
                }
            })
            .unwrap()
            .join()
            .expect("the run");
        let mut names: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        (names, std::fs::read_to_string(dir.join("index.csv")).expect("index.csv"))
    };
    let (a, ia) = one("a");
    let (b, ib) = one("b");
    assert!(a.len() > 3, "the menu shows 2D pictures ({:?})", a);
    assert_eq!(a, b, "the same frames, the same files");
    assert_eq!(ia, ib, "the same index");
    assert!(ia.lines().skip(1).all(|l| l.split(',').count() == 9), "nine columns in every row");
    assert!(a.iter().any(|n| n.starts_with("bg")), "background layers are dumped");
}

/// One run to `frames` VBlanks with the last frame drawn: its picture (RGBA, both screens) and, if `dump` is
/// given, the 2D dump of that frame; `pack` is the 2D pack.
fn run_2d(rom: &std::path::Path, frames: u64, pack: Option<PathBuf>, dump: Option<PathBuf>, shot: PathBuf) -> (usize, usize, Vec<u8>, usize) {
    let (rom, shot2) = (rom.to_path_buf(), shot.clone());
    let placed = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            let mut placed = 0;
            let mut cfg = Config::new(&rom);
            cfg.stop_vblanks = frames;
            cfg.input = (400..frames).step_by(100).map(|v| format!("{}:START", v)).collect::<Vec<_>>().join(",");
            cfg.pack_2d = pack;
            nds::boot(cfg).expect("boot");
            loop {
                nds::set_draw(nds::vblanks() + 1 == frames);
                if !nds::run_frame() {
                    break;
                }
                if nds::vblanks() == frames {
                    if let Some(d) = &dump {
                        nds::dump_2d(d);
                    }
                    nds::screenshot(&shot2).expect("screenshot");
                    placed = nds::pack_2d_placed();
                }
            }
            placed
        })
        .unwrap()
        .join()
        .expect("the run");
    let dec = png::Decoder::new(std::fs::File::open(&shot).expect("the picture"));
    let mut r = dec.read_info().expect("png");
    let mut buf = vec![0u8; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).expect("png frame");
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        _ => buf[..info.buffer_size()].chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
    };
    (info.width as usize, info.height as usize, rgba, placed)
}

/// The 2D pack (этап 8е.3): no pack and a pack of nothing draw the same picture; a pack made of the dump of a
/// frame draws that frame as it was; a pack of one-colour pictures changes the pixels of the sprites it names and
/// no others.
#[test]
fn a_2d_pack_replaces_the_sprites_it_names_and_nothing_else() {
    let Some(rom) = std::env::var_os("SUNPETAL_ROM").map(PathBuf::from) else {
        eprintln!("skipped: SUNPETAL_ROM is not set");
        return;
    };
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("pack2d");
    let _ = std::fs::remove_dir_all(&tmp);
    let (dump, empty, checker) = (tmp.join("dump"), tmp.join("empty"), tmp.join("checker"));
    std::fs::create_dir_all(&empty).unwrap();
    std::fs::create_dir_all(&checker).unwrap();
    // a frame whose sprites no blend or fade covers (the pack leaves those to the engine)
    let mut found = None;
    for frames in [1200u64, 1600, 2000, 2400, 3000, 3600] {
        let _ = std::fs::remove_dir_all(&dump);
        let (w, h, plain, _) = run_2d(&rom, frames, None, Some(dump.clone()), tmp.join("plain.png"));
        let Ok(index) = std::fs::read_to_string(dump.join("index.csv")) else { continue };
        if !index.lines().any(|l| l.starts_with("obj,")) {
            continue;
        }
        let (_, _, _, placed) = run_2d(&rom, frames, Some(dump.clone()), None, tmp.join("same.png"));
        if placed > 0 {
            found = Some((frames, w, h, plain));
            break;
        }
    }
    let (frames, w, h, plain) = found.expect("some frame shows sprites the pack can replace");
    let index = std::fs::read_to_string(dump.join("index.csv")).expect("index.csv");
    let objs: Vec<Vec<String>> = index.lines().skip(1).map(|l| l.split(',').map(str::to_owned).collect::<Vec<String>>()).filter(|c| c[0] == "obj").collect();
    assert!(!objs.is_empty(), "the frame at VBlank {} shows sprites", frames);
    let (_, _, none, _) = run_2d(&rom, frames, Some(empty), None, tmp.join("none.png"));
    assert!(plain == none, "a pack of nothing must not change the picture");
    // the pack of the dump: sprites are drawn over the engine's own, the picture is the same
    let (_, _, same, placed) = run_2d(&rom, frames, Some(dump.clone()), None, tmp.join("same.png"));
    let bad = plain.chunks_exact(4).zip(same.chunks_exact(4)).filter(|(a, b)| a != b).count();
    assert!(placed > 0);
    assert_eq!(bad, 0, "a pack of the dump changes {} pixels", bad);
    // one-colour pictures of the first sprite's size: only its pixels change
    let mut rects = Vec::new();
    for c in &objs {
        let (e, x, y, sw, sh): (usize, i32, i32, i32, i32) = (c[1].parse().unwrap(), c[3].parse().unwrap(), c[4].parse().unwrap(), c[5].parse().unwrap(), c[6].parse().unwrap());
        let name = &c[8];
        let mut px = Vec::new();
        for _ in 0..(sw * sh) {
            px.extend_from_slice(&[255, 0, 255, 255]);
        }
        let f = std::fs::File::create(checker.join(name)).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), sw as u32, sh as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&px).unwrap();
        rects.push((e, x, y, sw, sh));
    }
    let (_, _, painted, placed2) = run_2d(&rom, frames, Some(checker), None, tmp.join("checker.png"));
    assert!(placed2 > 0 && plain != painted, "the one-colour pack changes the picture");
    // both screens' rows: whichever engine is on top, a changed pixel lies in a sprite's rectangle of one of them
    let screen_h = h / 2;
    let mut changed = 0;
    for (i, (a, b)) in plain.chunks_exact(4).zip(painted.chunks_exact(4)).enumerate() {
        if a == b {
            continue;
        }
        changed += 1;
        let (x, y) = ((i % w) as i32, (i / w) as i32);
        let inside = rects.iter().any(|&(_, rx, ry, rw, rh)| {
            let yy = y % screen_h as i32;
            x >= rx && x < rx + rw && yy >= ry && yy < ry + rh
        });
        assert!(inside, "pixel ({}, {}) changed outside every sprite", x, y);
    }
    assert!(changed > 0);
}

/// The GPU's frame is read after the emulation has run on (этап 9.2.3-9.2.4): the picture at VBlank 2400
/// is the software engine's, drawn by the GPU throughout, and with the renderer switched back and forth
/// before it (a frame in flight never leaks into the next renderer's).
#[cfg(feature = "gpu")]
#[test]
fn the_gpu_frame_in_flight_is_read_before_anything_else() {
    let Some(rom) = std::env::var_os("SUNPETAL_ROM").map(PathBuf::from) else {
        eprintln!("skipped: SUNPETAL_ROM is not set");
        return;
    };
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("async3d");
    let mk = |name: &str| {
        let mut cfg = Config::new(&rom);
        cfg.out = tmp.join(name);
        cfg
    };
    let soft = picture_with(mk("soft"), 2400, tmp.join("soft.png"), nds::Renderer::Software, &[]);
    let gpu = picture_with(mk("gpu"), 2400, tmp.join("gpu.png"), nds::Renderer::Gpu, &[]);
    assert!(soft == gpu, "the GPU picture at VBlank 2400 differs from the software one");
    let mixed = picture_with(mk("mixed"), 2400, tmp.join("mixed.png"), nds::Renderer::Gpu, &[2000, 2200, 2300, 2399]);
    assert!(soft == mixed, "switching the renderer left a stale frame");
}

/// A GPU that fails when its frame is read (этап 9.2.5): the software engine draws the frame again from the
/// job it kept, so the picture is the software one's.
#[cfg(feature = "gpu")]
#[test]
fn a_gpu_frame_that_fails_in_flight_is_drawn_by_the_software_engine() {
    let Some(rom) = std::env::var_os("SUNPETAL_ROM").map(PathBuf::from) else {
        eprintln!("skipped: SUNPETAL_ROM is not set");
        return;
    };
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("async3d_fail");
    let mk = |name: &str| {
        let mut cfg = Config::new(&rom);
        cfg.out = tmp.join(name);
        cfg
    };
    let soft = picture_at(mk("soft"), 2400, tmp.join("soft.png"));
    let failing = picture_failing(mk("fail"), 2400, tmp.join("fail.png"), nds::Renderer::Gpu, &[], true);
    assert!(soft == failing, "a failed GPU frame was not redrawn by the software engine");
}
