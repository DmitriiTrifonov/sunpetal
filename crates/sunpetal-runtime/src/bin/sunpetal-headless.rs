//! Runs a cartridge's recompiled programs with no display or sound: the machine from the boot to
//! a VBlank limit, then a report. For the stage-3 runtime (docs/PLAN.md); the generated code of
//! the same ROM must be linked in (docs/ABI.md).
//!
//!     sunpetal-headless ROM.nds [--vblanks N] [--trace] [--out DIR] [--save FILE]
//!                              [--save-size BYTES] [--dump-textures DIR] [--dump-bg DIR] [--texture-pack DIR] [--shots V,V,...] [--shot-every N] [--input SCRIPT] [--wav FILE] [--scale N]
//!
//! Both screens are written as PNGs (DIR/v<N>.png) at the VBlanks asked for; --wav writes the sound
//! (16-bit stereo, 32 728 Hz); --draw-all draws every frame (for timing).
use std::process::ExitCode;

use sunpetal_runtime::nds::{self, Config};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut cfg: Option<Config> = None;
    let mut rest = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--vblanks" | "--out" | "--save" | "--save-size" | "--dump-textures" | "--dump-bg" | "--texture-pack" | "--shots" | "--shot-every" | "--input" | "--wav" | "--scale" => rest.push((a, args.next().unwrap_or_default())),
            "--trace" | "--draw-all" => rest.push((a, String::new())),
            _ if cfg.is_none() => cfg = Some(Config::new(a)),
            _ => {
                eprintln!("unknown argument {}", a);
                return ExitCode::from(2);
            }
        }
    }
    let Some(mut cfg) = cfg else {
        eprintln!("usage: sunpetal-headless ROM.nds [--vblanks N] [--trace] [--out DIR] [--save FILE]");
        return ExitCode::from(2);
    };
    let mut dump_bg: Option<std::path::PathBuf> = None;
    let (mut shots, mut shot_every): (Vec<u64>, u64) = (Vec::new(), 0);
    let mut wav: Option<String> = None;
    let mut draw_all = false;
    let mut scale = 1usize;
    for (k, v) in rest {
        match k.as_str() {
            "--vblanks" => cfg.stop_vblanks = v.parse().unwrap_or(0),
            "--out" => cfg.out = v.into(),
            "--dump-textures" => cfg.dump_textures = Some(v.into()),
            "--dump-bg" => dump_bg = Some(std::path::PathBuf::from(v)),
            "--texture-pack" => cfg.texture_pack = Some(v.into()),
            "--save-size" => cfg.save_size = v.parse().unwrap_or(8192),
            "--input" => cfg.input = v,
            "--scale" => scale = v.parse().unwrap_or(1),
            "--wav" => wav = Some(v),
            "--shots" => shots.extend(v.split(',').filter_map(|x| x.parse::<u64>().ok())),
            "--shot-every" => shot_every = v.parse().unwrap_or(0),
            "--save" => cfg.save = Some(v.into()),
            "--draw-all" => draw_all = true,
            _ => cfg.trace = true,
        }
    }
    let cfg_out = cfg.out.clone();
    if let Err(e) = nds::boot(cfg) {
        eprintln!("{}", e);
        return ExitCode::from(1);
    }
    let out = cfg_out;
    nds::set_scale(scale);
    nds::set_sound_keep(wav.is_some());
    let mut samples: Vec<i16> = Vec::new();
    let shot_at = |v: u64| (shot_every != 0 && v % shot_every == 0) || shots.contains(&v);
    let any_shots = !shots.is_empty() || shot_every != 0;
    loop {
        // headless, the 2D engines draw only what a picture shows
        nds::set_draw(draw_all || (any_shots && shot_at(nds::vblanks() + 1)));
        if !nds::run_frame() {
            break;
        }
        let v = nds::vblanks();
        if wav.is_some() {
            samples.extend(nds::take_samples());
        }
        if shot_at(v) {
            if let Some(d) = &dump_bg {
                nds::dump_backgrounds(d);
            }
            if let Err(e) = nds::screenshot(&out.join(format!("v{}.png", v))) {
                eprintln!("{}", e);
            }
        }
    }
    eprintln!("stopped: {}", nds::stopped().unwrap_or("by the host"));
    if let Some(path) = wav {
        if let Err(e) = write_wav(&path, &samples) {
            eprintln!("{}", e);
        }
    }
    nds::report();
    nds::finish();
    // the threads' coroutines are parked: do not unwind them
    std::process::exit(0)
}

/// 16-bit stereo PCM at the DS's sample rate (33 513 982 / 1024 Hz).
fn write_wav(path: &str, samples: &[i16]) -> Result<(), String> {
    let rate = 32_728u32;
    let data = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&2u16.to_le_bytes()); // stereo
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    if let Some(d) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(d);
    }
    std::fs::write(path, b).map_err(|e| format!("{}: {}", path, e))
}
