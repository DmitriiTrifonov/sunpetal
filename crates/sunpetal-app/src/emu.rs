//! The machine on its own thread: the runtime keeps its state per thread, and the window's thread
//! belongs to the window. Frames and sound go out, input comes in, through `Shared`.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sunpetal_runtime::nds::{self, Config};

use crate::audio::Ring;

/// A skip ends when the picture has not changed for this many frames (a second of the game's time),
/// or after this many in all (five minutes of it).
const STILL_FRAMES: u32 = 60;
const SKIP_LIMIT: u32 = 18_000;

/// A fingerprint of a picture from a sample of its pixels: it changes when the picture moves.
fn fingerprint(rgba: &[u8]) -> u64 {
    let step = (rgba.len() / 4 / 3000).max(1) * 4;
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for px in rgba.chunks_exact(4).step_by(step / 4) {
        for &b in &px[..3] {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

/// The DS's frame rate (33 513 982 Hz / (6 x 355 x 263 x 2 / 2)).
const FRAME_RATE: f64 = 59.8261;

#[derive(Default)]
pub struct Input {
    /// Buttons held, bit i of `nds::KEY_NAMES`.
    pub keys: u32,
    /// The stylus on the lower screen.
    pub pen: Option<(i32, i32)>,
}

pub struct Shared {
    pub input: Mutex<Input>,
    /// The latest picture: its scale, then both screens, RGBA, the upper first.
    pub frame: Mutex<Option<(usize, Vec<u8>)>>,
    /// The internal resolution, times 256x192 (1-8): the 3D is drawn that large.
    pub scale: AtomicUsize,
    pub quit: AtomicBool,
    /// Run without waiting for the clock.
    pub fast: AtomicBool,
    /// Skip: run without waiting until the picture stops changing (the game waits for the player).
    pub skip: AtomicBool,
    /// The machine waits (the menu is open).
    pub paused: AtomicBool,
    /// The emulation stopped (and why).
    pub stopped: Mutex<Option<String>>,
    /// A line about how it runs (DS frame, speed), for the screen.
    pub status: Mutex<String>,
}

impl Shared {
    pub fn new() -> Arc<Shared> {
        Arc::new(Shared {
            input: Mutex::new(Input::default()),
            frame: Mutex::new(None),
            scale: AtomicUsize::new(2),
            quit: AtomicBool::new(false),
            fast: AtomicBool::new(false),
            skip: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            stopped: Mutex::new(None),
            status: Mutex::new(String::from("starting")),
        })
    }
}

pub fn spawn(cfg: Config, shared: Arc<Shared>, audio: Option<Arc<Ring>>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("machine".into())
        .stack_size(64 << 20)
        .spawn(move || run(cfg, shared, audio))
        .expect("the machine's thread")
}

fn run(cfg: Config, shared: Arc<Shared>, audio: Option<Arc<Ring>>) {
    *shared.status.lock().unwrap() = "booting".into();
    if let Err(e) = nds::boot(cfg) {
        *shared.stopped.lock().unwrap() = Some(e);
        return;
    }
    nds::set_sound_keep(audio.is_some());
    let frame = Duration::from_secs_f64(1.0 / FRAME_RATE);
    let mut next = Instant::now();
    let stats = std::env::var_os("SUNPETAL_STATS").is_some();
    let (mut counted, mut since) = (0u32, Instant::now());
    let (mut shown, mut shown_since) = (0u32, Instant::now());
    let (mut t_run, mut t_grab) = (Duration::ZERO, Duration::ZERO);
    let mut prev = sunpetal_runtime::services::counters();
    let mut prev_t = nds::timing();
    let mut last_flush = Instant::now();
    let (mut still, mut skipped, mut last_print) = (0u32, 0u32, 0u64);
    while !shared.quit.load(Ordering::Relaxed) {
        if shared.paused.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(10));
            next = Instant::now();
            continue;
        }
        {
            let i = shared.input.lock().unwrap();
            nds::set_input(i.keys, i.pen);
        }
        nds::set_scale(shared.scale.load(Ordering::Relaxed));
        nds::set_draw(true);
        let t0 = Instant::now();
        if !nds::run_frame() {
            break;
        }
        let t1 = Instant::now();
        let picture = nds::screens_rgba();
        let skipping = shared.skip.load(Ordering::Relaxed);
        if skipping {
            let print = fingerprint(&picture.1);
            still = if print == last_print { still + 1 } else { 0 };
            last_print = print;
            skipped += 1;
            if still >= STILL_FRAMES || skipped >= SKIP_LIMIT {
                shared.skip.store(false, Ordering::Relaxed);
            }
        } else {
            (still, skipped) = (0, 0);
        }
        *shared.frame.lock().unwrap() = Some(picture);
        t_run += t1 - t0;
        t_grab += t1.elapsed();
        counted += 1;
        shown += 1;
        if shown_since.elapsed() >= Duration::from_secs(1) {
            let fps = shown as f64 / shown_since.elapsed().as_secs_f64();
            let c = sunpetal_runtime::services::counters();
            let tm = nds::timing();
            let per = |a: u64, b: u64| (a - b) as f64 / shown as f64;
            *shared.status.lock().unwrap() = format!(
                "DS frame {}, {:.0} fps\nrun {:.0} ms/frame, picture {:.1} ms\nper frame: polls {:.0}, calls {:.0}\nquanta {:.0}: guest {:.1} ms, hardware {:.1} ms, sound {:.1} ms",
                nds::vblanks(),
                fps,
                t_run.as_secs_f64() * 1000.0 / shown as f64,
                t_grab.as_secs_f64() * 1000.0 / shown as f64,
                per(c.0, prev.0),
                per(c.1, prev.1),
                (tm.0 - prev_t.0) as f64 / shown as f64,
                (tm.1 - prev_t.1) as f64 / 1e6 / shown as f64,
                (tm.2 - prev_t.2) as f64 / 1e6 / shown as f64,
                (tm.3 - prev_t.3) as f64 / 1e6 / shown as f64
            );
            if stats {
                eprintln!("{}", shared.status.lock().unwrap());
            }
            prev = c;
            prev_t = tm;
            t_run = Duration::ZERO;
            t_grab = Duration::ZERO;
            shown = 0;
            shown_since = Instant::now();
        }
        if last_flush.elapsed() >= Duration::from_secs(5) {
            nds::flush_save();
            last_flush = Instant::now();
        }
        if stats && since.elapsed() >= Duration::from_secs(2) {
            eprintln!("{:.1} frames/s (DS frame {})", counted as f64 / since.elapsed().as_secs_f64(), nds::vblanks());
            counted = 0;
            since = Instant::now();
        }
        let samples = nds::take_samples();
        if let Some(a) = &audio {
            if !skipping {
                a.push(&samples);
            }
        }
        if shared.fast.load(Ordering::Relaxed) || skipping {
            next = Instant::now();
            continue;
        }
        next += frame;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > Duration::from_millis(100) {
            next = now; // far behind: do not run to catch up
        }
    }
    let why = nds::stopped().unwrap_or("closed");
    *shared.stopped.lock().unwrap() = Some(why.to_string());
    nds::report();
    nds::finish();
    // the guest threads' coroutines are parked: do not unwind them
    std::process::exit(0);
}

#[allow(dead_code)]
pub fn default_out(rom: &std::path::Path) -> PathBuf {
    rom.parent().map(|p| p.to_path_buf()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(seed: u8) -> Vec<u8> {
        (0..2 * 256 * 192 * 4).map(|i| if i % 4 == 3 { 255 } else { (i as u32 * 7 + seed as u32 * 13) as u8 }).collect()
    }

    #[test]
    fn a_still_picture_has_one_fingerprint_and_a_moving_one_another() {
        assert_eq!(fingerprint(&picture(1)), fingerprint(&picture(1)));
        assert_ne!(fingerprint(&picture(1)), fingerprint(&picture(2)));
        // a change of one pixel in the middle of a row the sample takes
        let mut a = picture(1);
        let before = fingerprint(&a);
        let step = (a.len() / 4 / 3000).max(1) * 4;
        a[step * 100] ^= 0x40;
        assert_ne!(before, fingerprint(&a));
    }

    #[test]
    fn a_skip_is_long_enough_to_see_a_pause_and_short_enough_to_stop() {
        // a second of the game's time still, and never more than five minutes
        assert_eq!(STILL_FRAMES, 60);
        assert_eq!(SKIP_LIMIT / 60, 300);
    }
}
