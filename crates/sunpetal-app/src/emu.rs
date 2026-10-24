//! The machine on its own thread: the runtime keeps its state per thread, and the window's thread
//! belongs to the window. Frames and sound go out, input comes in, through `Shared`.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sunpetal_runtime::nds::{self, Config};

use crate::audio::Ring;

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
    let mut last_flush = Instant::now();
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
        *shared.frame.lock().unwrap() = Some(nds::screens_rgba());
        t_run += t1 - t0;
        t_grab += t1.elapsed();
        counted += 1;
        shown += 1;
        if shown_since.elapsed() >= Duration::from_secs(1) {
            let fps = shown as f64 / shown_since.elapsed().as_secs_f64();
            let c = sunpetal_runtime::services::counters();
            let per = |a: u64, b: u64| (a - b) as f64 / shown as f64;
            *shared.status.lock().unwrap() = format!(
                "DS frame {}, {:.0} fps\nrun {:.0} ms/frame, picture {:.1} ms\nper frame: polls {:.0}\ncalls {:.0}, interpreted {:.0}",
                nds::vblanks(),
                fps,
                t_run.as_secs_f64() * 1000.0 / shown as f64,
                t_grab.as_secs_f64() * 1000.0 / shown as f64,
                per(c.0, prev.0),
                per(c.1, prev.1),
                per(c.2, prev.2)
            );
            if stats {
                eprintln!("{}", shared.status.lock().unwrap());
            }
            prev = c;
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
            a.push(&samples);
        }
        if shared.fast.load(Ordering::Relaxed) {
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
