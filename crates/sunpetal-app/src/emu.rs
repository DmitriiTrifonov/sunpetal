//! The machine on its own thread: the runtime keeps its state per thread, and the window's thread
//! belongs to the window. Frames and sound go out, input comes in, through `Shared`.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sunpetal_runtime::nds::{self, Config};

use crate::audio::Ring;

/// A skip ends when the picture has not changed for this many frames (a second of the game's time),
/// or after this many in all (five minutes of it).
const STILL_FRAMES: u32 = 60;
const SKIP_LIMIT: u32 = 18_000;
/// With the game's own marks (`nds::script_idle`, `nds::script_waits`), a skip runs the scene on, undrawn, until the
/// script waits for the player's A (a line of speech, a `WAIT`) or has been idle this long (so a gap between two
/// scenes does not end it). It presses nothing: the lines are the player's.
const IDLE_FRAMES: u32 = 45;
/// A skip ends when the script has made no step (the command it is at, the count of a wait) for this many frames:
/// the game waits for something the script does not name (a menu, a report page), and nothing would end the skip.
const NO_STEP_FRAMES: u32 = 300;

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

/// A dialogue box at the foot of the upper screen: a pure red rule across it, grey panel below.
/// The game waits for the player there, so a skip has nothing to cross.
fn dialogue_shown(scale: usize, rgba: &[u8]) -> bool {
    let width = 256 * scale;
    let at = |x: usize, y: usize| {
        let o = (y * width + x) * 4;
        rgba.get(o..o + 3).map_or([0; 3], |p| [p[0], p[1], p[2]])
    };
    let rule = 143 * scale + scale / 2;
    let red = (16..240).step_by(4).filter(|&x| at(x * scale, rule) == [255, 0, 0]).count();
    let [r, g, b] = at(128 * scale, 165 * scale);
    red >= 50 && r > 120 && r.abs_diff(g) < 16 && r.abs_diff(b) < 16
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

/// The game's first screens, a black "Licensed by Nintendo" notice, end by this VBlank (the studio
/// logos begin at ~230). Nothing here is licensed by Nintendo, so it is run through unseen.
const NOTICE_END_VBLANKS: u64 = 215;

/// The vblank at which the logos at the start end (the screens go black, then the title): the game
/// takes no input before it (A, START and the stylus change nothing), so a click races through them.
const LOGOS_END_VBLANKS: u64 = 580;

pub struct Shared {
    pub input: Mutex<Input>,
    /// The latest picture: its scale, then both screens, RGBA, the upper first.
    pub frame: Mutex<Option<(usize, Vec<u8>)>>,
    /// The internal resolution, times 256x192 (1-8): the 3D is drawn that large.
    pub scale: AtomicUsize,
    /// The engine that draws the 3D: 0 software, 1 the GPU one (`Renderer`), changed by the window.
    pub renderer: AtomicU8,
    /// The 3D's outline and shading.
    pub extras: Mutex<nds::Extras>,
    /// A texture pack to swap in before the next frame (`Some(None)`: none); the window sets it, the emulation takes it.
    pub pack_swap: Mutex<Option<Option<PathBuf>>>,
    /// A click (a tap) came while the logos were on: run on, without waiting or sound, until they end.
    pub logo_click: AtomicBool,
    pub quit: AtomicBool,
    /// How many DS pixels the 3D view is widened by each side (the window's shape asks, 0: none),
    /// and the upper screen so widened with its scale and that width (set by the emulation with the
    /// picture, `None` when the 3D is not on it).
    pub wide_px: AtomicUsize,
    pub wide_frame: Mutex<Option<(usize, usize, Vec<u8>)>>,
    /// The DS frame the machine is at (set by the emulation each frame).
    pub vblank: AtomicU64,
    /// Run without waiting for the clock.
    pub fast: AtomicBool,
    /// Skip: run without waiting until the picture stops changing (the game waits for the player).
    pub skip: AtomicBool,
    /// Write the picture and its parts to artifacts/shots (F10).
    pub dump: AtomicBool,
    /// What the window knows of the picture, as the inside of a JSON object, for the dump's file (set with `dump`).
    pub dump_meta: Mutex<String>,
    /// A dialogue box is on the screen (set by the emulation each frame).
    pub dialogue: AtomicBool,
    /// The text of the dialogue line the game has begun to draw (set with the picture, `None` when the window shows none).
    pub dialogue_text: Mutex<Option<(String, Option<f32>)>>,
    /// The window draws the dialogue text itself (set by the window each frame): the game's raster text is hidden.
    pub crisp_text: AtomicBool,
    /// The game says when its script runs, so a skip can go through dialogues (set by the emulation).
    pub marks: AtomicBool,
    /// The script is idle (the player has the control): there is nothing to skip (set by the emulation).
    pub idle: AtomicBool,
    /// The script waits for the player's A (set by the emulation).
    pub waiting: AtomicBool,
    /// The machine waits (the menu is open).
    pub paused: AtomicBool,
    /// The emulation stopped (and why).
    pub stopped: Mutex<Option<String>>,
    /// A line about how it runs (DS frame, speed), for the screen.
    pub status: Mutex<String>,
}

impl Shared {
    /// A skip starts, or a running one is cancelled; none starts while the game's script is idle (nothing to skip: the
    /// title, a free walk) or waits for the player (a line of speech), and without the game's marks none starts over a dialogue.
    pub fn toggle_skip(&self) {
        let on = !self.skip.load(Ordering::Relaxed);
        let marks = self.marks.load(Ordering::Relaxed);
        let blocked = (self.dialogue.load(Ordering::Relaxed) && !marks) || (marks && (self.idle.load(Ordering::Relaxed) || self.waiting.load(Ordering::Relaxed)));
        self.skip.store(on && !blocked, Ordering::Relaxed);
    }

    pub fn new() -> Arc<Shared> {
        Arc::new(Shared {
            input: Mutex::new(Input::default()),
            frame: Mutex::new(None),
            scale: AtomicUsize::new(2),
            renderer: AtomicU8::new(0),
            extras: Mutex::new(nds::Extras::default()),
            pack_swap: Mutex::new(None),
            logo_click: AtomicBool::new(false),
            quit: AtomicBool::new(false),
            vblank: AtomicU64::new(0),
            wide_px: AtomicUsize::new(0),
            dump: AtomicBool::new(false),
            dump_meta: Mutex::new(String::new()),
            wide_frame: Mutex::new(None),
            fast: AtomicBool::new(false),
            skip: AtomicBool::new(false),
            dialogue: AtomicBool::new(false),
            dialogue_text: Mutex::new(None),
            crisp_text: AtomicBool::new(false),
            marks: AtomicBool::new(false),
            idle: AtomicBool::new(false),
            waiting: AtomicBool::new(false),
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
    // the side of the dialogue portrait and the panel's middle, last seen (the window draws the text with them)
    let mut portrait_left = true;
    let mut middle: Option<f32> = None;
    let (mut counted, mut since) = (0u32, Instant::now());
    let (mut shown, mut shown_since) = (0u32, Instant::now());
    let (mut t_run, mut t_grab) = (Duration::ZERO, Duration::ZERO);
    let mut prev = sunpetal_runtime::services::counters();
    let mut prev_t = nds::timing();
    let mut last_flush = Instant::now();
    nds::track_script();
    let (mut still, mut skipped, mut last_print, mut idle_for, mut wait_for) = (0u32, 0u32, 0u64, 0u32, 0u32);
    let (mut step, mut no_step) = ((0u32, 0u32), 0u32);
    let mut racing;
    let mut last_e = usize::MAX;
    // drawn frames since the 3D last gave a widened picture (a fade, a film: the wide view is kept a while)
    let mut no_wide = u32::MAX;
    // frames of real wide picture in a row before (a short one, as the logos' few, is no scene to keep)
    let mut wide_streak = 0u32;
    while !shared.quit.load(Ordering::Relaxed) {
        if shared.paused.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(10));
            next = Instant::now();
            continue;
        }
        racing = shared.logo_click.load(Ordering::Relaxed);
        if racing && nds::vblanks() >= LOGOS_END_VBLANKS {
            shared.logo_click.store(false, Ordering::Relaxed);
            racing = false;
        }
        {
            let i = shared.input.lock().unwrap();
            nds::set_input(i.keys, i.pen);
        }
        // with the game's marks a skip draws nothing (the screen is dark for it: the frames cost only the game's
        // own time); without them it needs the picture, at the native size, the cheapest
        let notice = nds::vblanks() < NOTICE_END_VBLANKS;
        let skipping_now = shared.skip.load(Ordering::Relaxed) || racing || notice;
        let draw = !notice && !(skipping_now && shared.marks.load(Ordering::Relaxed));
        let want_e = shared.wide_px.load(Ordering::Relaxed);
        if want_e != last_e {
            nds::set_wide(want_e);
            last_e = want_e;
        }
        nds::set_renderer(if shared.renderer.load(Ordering::Relaxed) == 1 { nds::Renderer::Gpu } else { nds::Renderer::Software });
        nds::set_scale(if skipping_now { 1 } else { shared.scale.load(Ordering::Relaxed) });
        let mut extras = *shared.extras.lock().unwrap();
        // the window's 3D is drawn at scale x samples at most (`effects::MSAA_BUDGET`, the video card's is larger): more would not keep up
        extras.msaa = crate::effects::effective_msaa(shared.scale.load(Ordering::Relaxed) as u32, extras.msaa, shared.renderer.load(Ordering::Relaxed) == 1);
        nds::set_extras(extras);
        if let Some(pack) = shared.pack_swap.lock().unwrap().take() {
            if let Err(e) = nds::set_texture_pack(pack.as_deref()) {
                eprintln!("texture set: {}", e);
            }
        }
        nds::set_draw(draw);
        let t0 = Instant::now();
        if !nds::run_frame() {
            break;
        }
        if shared.dump.swap(false, Ordering::Relaxed) {
            let stem = std::path::PathBuf::from(format!("artifacts/shots/v{}", nds::vblanks()));
            if let Some(d) = stem.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = nds::screenshot(&std::path::PathBuf::from(format!("{}.png", stem.display())));
            let _ = nds::screenshot_wide_layers(&stem);
            let json = format!(
                "{{\"vblank\":{},\"scene\":{},\"script\":{{\"idle\":{},\"waits\":{},\"mark\":{},\"step\":{}}},\"frame_scale\":{},{}}}\n",
                nds::vblanks(),
                nds::script_scene().map_or("null".to_string(), |v| format!("{:?}", v)),
                json_opt(nds::script_idle()),
                json_opt(nds::script_waits()),
                json_opt(nds::script_mark()),
                json_opt(nds::script_step().map(|(a, b)| [a, b])),
                shared.scale.load(Ordering::Relaxed),
                shared.dump_meta.lock().unwrap()
            );
            let _ = std::fs::write(format!("{}.json", stem.display()), json);
        }
        let t1 = Instant::now();
        let skipping = shared.skip.load(Ordering::Relaxed);
        let idle = nds::script_idle();
        shared.marks.store(idle.is_some(), Ordering::Relaxed);
        shared.idle.store(idle == Some(true), Ordering::Relaxed);
        let waits = nds::script_waits() == Some(true);
        shared.waiting.store(waits, Ordering::Relaxed);
        if draw {
            let picture = nds::screens_rgba();
            shared.dialogue.store(dialogue_shown(picture.0, &picture.1), Ordering::Relaxed);
            if let Some(left) = crate::layout::portrait_left(picture.0, &picture.1) {
                portrait_left = left;
            }
            // the panel's middle is kept while the red line is out of sight for a frame (the box opens and closes)
            if let Some(m) = crate::layout::dialogue_panel_middle(picture.0, &picture.1) {
                middle = Some(m);
            }
            let dialogue = nds::dialogue();
            // the game's raster text is hidden from the moment a line is set, before its letters come; ours shows when they do
            nds::hide_dialogue_raster(shared.crisp_text.load(Ordering::Relaxed) && dialogue.as_ref().is_some_and(|d| d.coming));
            let dialogue_line = dialogue.filter(|d| d.drawn).map(|d| (d.text, middle));
            if skipping {
                let print = fingerprint(&picture.1);
                still = if print == last_print { still + 1 } else { 0 };
                last_print = print;
            }
            // the widened picture first: the window takes the plain one and then looks for its wide partner
            let mut wide = if want_e > 0 { nds::upper_wide_rgba() } else { None };
            no_wide = if wide.is_some() { 0 } else { no_wide.saturating_add(1) };
            wide_streak = if wide.is_some() { wide_streak + 1 } else if no_wide > WIDE_KEEP_FRAMES { 0 } else { wide_streak };
            // no 3D for a moment: the plain upper screen, its edge columns run on, so the view does not
            // jump between wide and narrow
            if wide.is_none() && want_e > 0 && no_wide <= WIDE_KEEP_FRAMES && wide_streak >= 30 {
                wide = Some(extend_edges(picture.0, want_e, &picture.1));
            }
            // both under the picture's lock, so the window takes the pair or neither
            let mut slot = shared.frame.lock().unwrap();
            *shared.wide_frame.lock().unwrap() = wide;
            *shared.dialogue_text.lock().unwrap() = dialogue_line;
            *slot = Some(picture);
            drop(slot);
        }
        if skipping {
            skipped += 1;
            idle_for = if idle == Some(true) { idle_for + 1 } else { 0 };
            wait_for = if waits { wait_for + 1 } else { 0 };
            let now = nds::script_step().unwrap_or((0, 0));
            no_step = if now == step { no_step + 1 } else { 0 };
            step = now;
            // a still picture means the game waits, which only a skip without the game's marks can tell
            let stuck = idle.is_none() && still >= STILL_FRAMES;
            if idle_for >= IDLE_FRAMES || wait_for >= 2 || no_step >= NO_STEP_FRAMES || stuck || skipped >= SKIP_LIMIT {
                shared.skip.store(false, Ordering::Relaxed);
            }
        } else {
            (still, skipped, idle_for, wait_for, no_step) = (0, 0, 0, 0, 0);
        }
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
                "DS frame {}, {:.0} fps\nrun {:.0} ms/frame, picture {:.1} ms\nper frame: polls {:.0}, calls {:.0}\nquanta {:.0}: guest {:.1} ms, hardware {:.1} ms, sound {:.1} ms\nskip {} (marks {}, idle {}, waiting {}, loaded {}, dialogue {}), script mark {:08X}",
                nds::vblanks(),
                fps,
                t_run.as_secs_f64() * 1000.0 / shown as f64,
                t_grab.as_secs_f64() * 1000.0 / shown as f64,
                per(c.0, prev.0),
                per(c.1, prev.1),
                (tm.0 - prev_t.0) as f64 / shown as f64,
                (tm.1 - prev_t.1) as f64 / 1e6 / shown as f64,
                (tm.2 - prev_t.2) as f64 / 1e6 / shown as f64,
                (tm.3 - prev_t.3) as f64 / 1e6 / shown as f64,
                skipping,
                shared.marks.load(Ordering::Relaxed),
                shared.idle.load(Ordering::Relaxed),
                shared.waiting.load(Ordering::Relaxed),
                nds::script_loaded().unwrap_or(false),
                shared.dialogue.load(Ordering::Relaxed),
                nds::script_mark().unwrap_or(0)
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
        shared.vblank.store(nds::vblanks(), Ordering::Relaxed);
        let samples = nds::take_samples();
        if let Some(a) = &audio {
            if !skipping && !racing && !notice {
                a.push(&samples);
            }
        }
        if shared.fast.load(Ordering::Relaxed) || skipping || racing || notice {
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
    fn a_dialogue_box_is_a_red_rule_over_a_grey_panel() {
        let mut p = vec![255u8; 2 * 256 * 192 * 4];
        for px in p.chunks_exact_mut(4) {
            px[..3].copy_from_slice(&[40, 90, 30]);
        }
        assert!(!dialogue_shown(1, &p));
        for x in 0..256 {
            p[(143 * 256 + x) * 4..][..3].copy_from_slice(&[255, 0, 0]);
        }
        assert!(!dialogue_shown(1, &p));
        for y in 144..192 {
            for x in 0..256 {
                p[(y * 256 + x) * 4..][..3].copy_from_slice(&[170, 170, 170]);
            }
        }
        assert!(dialogue_shown(1, &p));
    }

    #[test]
    fn a_skip_is_long_enough_to_see_a_pause_and_short_enough_to_stop() {
        // a second of the game's time still, and never more than five minutes
        assert_eq!(STILL_FRAMES, 60);
        assert_eq!(SKIP_LIMIT / 60, 300);
    }
}

#[cfg(test)]
mod skip_tests {
    use super::*;

    /// F7 on the title screen starts nothing (the game's script is idle there). Needs the
    /// ROM (`SUNPETAL_ROM`) and runs the machine for real: skipped without it.
    #[test]
    fn a_skip_on_the_title_screen_does_not_start() {
        let Some(rom) = std::env::var_os("SUNPETAL_ROM").map(PathBuf::from) else {
            eprintln!("skipped: SUNPETAL_ROM is not set");
            return;
        };
        let mut cfg = Config::new(&rom);
        cfg.out = std::env::temp_dir();
        cfg.save = Some(std::env::temp_dir().join("skip_title.sav"));
        let shared = Shared::new();
        shared.fast.store(true, Ordering::Relaxed);
        let _machine = spawn(cfg, shared.clone(), None);
        let frame_of = |s: &Shared| -> u64 { s.status.lock().unwrap().split_whitespace().nth(2).and_then(|n| n.trim_end_matches(',').parse().ok()).unwrap_or(0) };
        let t = Instant::now();
        while frame_of(&shared) < 2700 && t.elapsed() < Duration::from_secs(120) {
            std::thread::sleep(Duration::from_millis(100));
        }
        let at = frame_of(&shared);
        assert!(at >= 2700, "the game did not reach the title: frame {}", at);
        eprintln!("marks {}", shared.marks.load(Ordering::Relaxed));
        // the title has no script running: F7 does nothing, the screen does not go dark
        shared.toggle_skip();
        assert!(!shared.skip.load(Ordering::Relaxed), "a skip started with nothing to skip");
        std::thread::sleep(Duration::from_millis(500));
        let (_, rgba) = shared.frame.lock().unwrap().clone().expect("a picture");
        let bright = rgba.chunks_exact(4).map(|p| p[0] as u64 + p[1] as u64 + p[2] as u64).sum::<u64>() / (rgba.len() as u64 / 4);
        assert!(bright > 30, "the picture is black");
        shared.quit.store(true, Ordering::Relaxed);
    }
}

/// Drawn frames the wide view stays after the 3D stops giving a widened picture.
const WIDE_KEEP_FRAMES: u32 = 90;

/// The upper screen of `rgba` (both screens, `scale` times 256x192 each) `each_side` DS pixels wider on
/// each side, the new columns the edge ones repeated.
fn extend_edges(scale: usize, each_side: usize, rgba: &[u8]) -> (usize, usize, Vec<u8>) {
    let (w, h, e) = (256 * scale, 192 * scale, each_side * scale);
    let mut out = Vec::with_capacity((w + 2 * e) * h * 4);
    for row in rgba[..w * h * 4].chunks_exact(w * 4) {
        for _ in 0..e {
            out.extend_from_slice(&row[..4]);
        }
        out.extend_from_slice(row);
        for _ in 0..e {
            out.extend_from_slice(&row[row.len() - 4..]);
        }
    }
    (scale, each_side, out)
}

/// An optional number, flag or list as JSON.
fn json_opt<T: std::fmt::Debug>(v: Option<T>) -> String {
    v.map_or("null".to_string(), |v| format!("{:?}", v))
}
