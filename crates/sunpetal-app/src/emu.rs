//! The machine on its own thread: the runtime keeps its state per thread, and the window's thread
//! belongs to the window. Frames and sound go out, input comes in, through `Shared`.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// The latest picture: both screens, RGBA, the upper first.
    pub frame: Mutex<Option<Vec<u8>>>,
    pub quit: AtomicBool,
    /// Run without waiting for the clock.
    pub fast: AtomicBool,
    /// The emulation stopped (and why).
    pub stopped: Mutex<Option<String>>,
}

impl Shared {
    pub fn new() -> Arc<Shared> {
        Arc::new(Shared {
            input: Mutex::new(Input::default()),
            frame: Mutex::new(None),
            quit: AtomicBool::new(false),
            fast: AtomicBool::new(false),
            stopped: Mutex::new(None),
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
    if let Err(e) = nds::boot(cfg) {
        *shared.stopped.lock().unwrap() = Some(e);
        return;
    }
    nds::set_sound_keep(audio.is_some());
    let frame = Duration::from_secs_f64(1.0 / FRAME_RATE);
    let mut next = Instant::now();
    let stats = std::env::var_os("SUNPETAL_STATS").is_some();
    let (mut counted, mut since) = (0u32, Instant::now());
    while !shared.quit.load(Ordering::Relaxed) {
        {
            let i = shared.input.lock().unwrap();
            nds::set_input(i.keys, i.pen);
        }
        nds::set_draw(true);
        if !nds::run_frame() {
            break;
        }
        *shared.frame.lock().unwrap() = Some(nds::screens_rgba());
        counted += 1;
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
