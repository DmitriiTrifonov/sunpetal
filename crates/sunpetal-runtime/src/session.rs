//! A game run driven from the host, a command at a time (feature `debug-tools`): the base of the
//! proofreading tools (docs/DEBUG-TOOLS-PLAN.md). Time is counted in frames, so a run of the same
//! commands gives the same pictures.
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::dialogue::{self, Shown, Tracker};
use crate::nds::{self, debug, Config, KEY_NAMES};
use crate::nitrofs::NitroFs;

/// A dialogue window opened (or its line replaced): one line of the dialogue log.
#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub frame: u64,
    pub scene: Option<String>,
    /// The line's id as the script has it, five digits.
    pub id: Option<String>,
    /// The speaker's face number (`SETFACE2`).
    pub face: Option<u32>,
    /// Which side the portrait is on in the window (null: none), and the columns the text takes: from
    /// `text_x0`, up to `text_x1` (the game clips there).
    pub portrait: Option<&'static str>,
    pub text_x0: u32,
    pub text_x1: u32,
    pub text: String,
    pub lines: usize,
    /// Each line's width in pixels as the game's font measures it.
    pub widths: Vec<u32>,
    /// More than 3 lines, or a line wider than 194 px.
    pub overflow: bool,
}

pub struct Session {
    /// Buttons and the stylus now held by `press` and `tap`.
    keys: u32,
    pen: Option<(i32, i32)>,
    tracker: Tracker,
    fs: NitroFs,
    rom: PathBuf,
    /// The scene (`Cxx_yy`) whose script the game read last.
    scene: Option<String>,
    shown: Option<(u32, String)>,
    /// The dialogue log, line by line, flushed at each line.
    log: Option<std::io::BufWriter<std::fs::File>>,
    events: Vec<Event>,
    texts: Option<Texts>,
    frames: u64,
    /// The events `silence_rivals` put out: the address of the record and the first byte of its name.
    silenced: Vec<(u32, u8)>,
    /// The lines shown while the chapter's opening plays to a scene are not written to the log (it was added to:
    /// the run before wrote them).
    quiet_opening: bool,
    quiet: bool,
}

/// Lines in a row, all shown before in a scene, after which autoplay counts the scene as stalled.
const REPEATS: usize = 6;

/// The lines of a scene shown so far, and how many in a row were shown before.
#[derive(Default)]
struct Repeats {
    seen: std::collections::HashSet<Option<String>>,
    run: usize,
}

impl Repeats {
    /// Takes the next line shown; true once `REPEATS` of them in a row were seen already.
    fn stalled(&mut self, id: &Option<String>) -> bool {
        self.run = if self.seen.insert(id.clone()) { 0 } else { self.run + 1 };
        self.run >= REPEATS
    }
}

/// The mask of `+`-joined button names (A B SELECT START RIGHT LEFT UP DOWN R L X Y).
pub fn buttons(names: &str) -> Result<u32, String> {
    let mut mask = 0;
    for n in names.split('+').filter(|n| !n.is_empty() && *n != "-") {
        let bit = KEY_NAMES.iter().position(|k| k.eq_ignore_ascii_case(n)).ok_or_else(|| format!("no button {}", n))?;
        mask |= 1 << bit;
    }
    Ok(mask)
}

impl Session {
    /// Boots `rom` with its save in `save` (a 64 KiB EEPROM, made if not there).
    pub fn open(rom: impl Into<PathBuf>, save: Option<PathBuf>) -> Result<Session, String> {
        Session::open_with(rom, save, None)
    }

    /// The same with `overlay`, a folder of files that replace the ROM's (`nitrofs::apply_overlay`):
    /// the files are named by the image the game reads, the overlaid one.
    pub fn open_with(rom: impl Into<PathBuf>, save: Option<PathBuf>, overlay: Option<PathBuf>) -> Result<Session, String> {
        let rom = rom.into();
        let mut image = std::fs::read(&rom).map_err(|e| format!("{}: {}", rom.display(), e))?;
        if let Some(dir) = &overlay {
            crate::nitrofs::apply_overlay(&mut image, &crate::nitrofs::read_overlay_dir(dir)?)?;
        }
        let fs = NitroFs::parse(&image)?;
        let rom_path = rom.clone();
        let mut cfg = Config::new(rom);
        cfg.overlay = overlay;
        cfg.save = save;
        cfg.save_size = 65536;
        nds::boot(cfg)?;
        nds::set_sound_keep(false);
        debug::log_card_reads();
        nds::track_script();
        let tracker = Tracker::install();
        Ok(Session { keys: 0, pen: None, tracker, fs, rom: rom_path, scene: None, shown: None, log: None, events: Vec::new(), texts: None, frames: 0, silenced: Vec::new(), quiet_opening: false, quiet: false })
    }

    pub fn vblank(&self) -> u64 {
        nds::vblanks()
    }

    /// Runs `n` frames with the input as it is; an error if the game stopped.
    pub fn run(&mut self, n: u64) -> Result<(), String> {
        for _ in 0..n {
            nds::set_input(self.keys, self.pen);
            if !nds::run_frame() {
                return Err(format!("the game stopped: {}", nds::stopped().unwrap_or("?")));
            }
            self.watch()?;
            self.frames += 1;
            if self.texts.is_some() && self.frames % 30 == 0 {
                self.apply_texts()?;
            }
        }
        Ok(())
    }

    /// After a frame: the scene the game reads, and the line the window shows.
    fn watch(&mut self) -> Result<(), String> {
        if let Some(scene) = nds::script_scene() {
            self.scene = Some(scene);
        }
        let now = self.tracker.shown();
        let key = now.as_ref().map(|l| (l.ptr, l.text.clone()));
        // the same slot of the same line with other bytes in it: the game has taken the buffer for something
        // else while the window is still open, and what it holds is no text of the line
        let reused = match (&now, &self.shown, self.events.last()) {
            (Some(l), Some((ptr, _)), Some(last)) => l.ptr == *ptr && l.id.is_some() && l.id.map(|i| format!("{:05}", i)) == last.id,
            _ => false,
        };
        if key != self.shown && !reused {
            self.shown = key;
            // a text the scripts do not own (no id) is a stale pointer, not a window
            if let Some(line) = now.filter(|l| l.id.is_some()) {
                let event = self.event(line);
                if let Some(log) = self.log.as_mut().filter(|_| !self.quiet) {
                    let json = serde_json::to_string(&event).map_err(|e| e.to_string())?;
                    writeln!(log, "{}", json).and_then(|_| log.flush()).map_err(|e| format!("dialogue log: {}", e))?;
                }
                self.events.push(event);
            }
        }
        Ok(())
    }

    fn event(&self, line: Shown) -> Event {
        let widths = dialogue::line_widths(&line.text).unwrap_or_default();
        let lines = line.text.split('\n').count();
        let overflow = lines > dialogue::MAX_LINES || widths.iter().any(|w| *w > dialogue::MAX_WIDTH);
        Event { frame: nds::vblanks(), scene: self.scene.clone(), id: line.id.map(|i| format!("{:05}", i)), face: line.face, portrait: line.portrait.then_some("left"), text_x0: dialogue::TEXT_X0, text_x1: dialogue::TEXT_X1, text: line.text, lines, widths, overflow }
    }

    /// Starts writing the dialogue log to `path` (one JSON line per line shown), or stops it.
    pub fn log_to(&mut self, path: Option<&Path>, append: bool) -> Result<(), String> {
        self.quiet_opening = append;
        self.log = match path {
            Some(p) => {
                if let Some(d) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
                    std::fs::create_dir_all(d).map_err(|e| format!("{}: {}", d.display(), e))?;
                }
                let file = if append { std::fs::OpenOptions::new().create(true).append(true).open(p) } else { std::fs::File::create(p) };
                Some(std::io::BufWriter::new(file.map_err(|e| format!("{}: {}", p.display(), e))?))
            }
            None => None,
        };
        Ok(())
    }

    /// The lines shown so far.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Presses A every 60 frames until the window shows another line, for at most `max` frames. The line, if
    /// there is one.
    pub fn advance(&mut self, max: u64) -> Result<Option<Event>, String> {
        let before = self.events.len();
        // a line that is still being written is finished by the first A, the next one turns it
        for _ in 0..max.div_ceil(60) {
            self.press("A", 5)?;
            self.run(55)?;
            if self.events.len() > before {
                break;
            }
        }
        Ok(self.events.get(before).cloned())
    }

    /// Holds `names` for `frames` frames, then lets go.
    pub fn press(&mut self, names: &str, frames: u64) -> Result<(), String> {
        self.keys = buttons(names)?;
        let r = self.run(frames);
        self.keys = 0;
        r
    }

    /// Holds the stylus at (x, y) of the lower screen for `frames` frames, then lifts it.
    pub fn tap(&mut self, x: i32, y: i32, frames: u64) -> Result<(), String> {
        if !(0..256).contains(&x) || !(0..192).contains(&y) {
            return Err(format!("tap {} {}: the lower screen is 256 x 192", x, y));
        }
        self.pen = Some((x, y));
        let r = self.run(frames);
        self.pen = None;
        r
    }

    /// Both screens of the next frame (it is run: only such frames are drawn) as a PNG.
    pub fn shot(&mut self, path: &Path) -> Result<(), String> {
        nds::set_draw(true);
        let r = self.run(1);
        nds::set_draw(false);
        r?;
        if let Some(d) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {}", d.display(), e))?;
        }
        nds::screenshot(path)
    }

    /// The functions entered in the next `n` frames, by (cpu, key): the count and the first
    /// entry. At most `cap` entries are recorded per frame; `names` are held for the first 9.
    pub fn calls(&mut self, n: u64, cap: usize, names: &str) -> Result<Vec<(u64, debug::Call)>, String> {
        let mut seen: HashMap<(i32, u32), (u64, debug::Call)> = HashMap::new();
        let held = buttons(names)?;
        for i in 0..n {
            self.keys = if i < 9 { held } else { 0 };
            debug::log_calls(cap);
            let r = self.run(1);
            for c in debug::take_calls() {
                seen.entry((c.cpu, c.key)).or_insert((0, c)).0 += 1;
            }
            debug::log_calls(0);
            r?;
        }
        self.keys = 0;
        let mut rows: Vec<_> = seen.into_values().collect();
        rows.sort_by_key(|(_, c)| (c.vblank, c.cpu, c.key));
        Ok(rows)
    }
}

impl Session {
    /// The function entries of the next `n` frames that have a value of `value` among their four arguments
    /// (`names` held for the first 9 frames), in order.
    pub fn calls_with(&mut self, n: u64, value: std::ops::RangeInclusive<u32>, names: &str) -> Result<Vec<debug::Call>, String> {
        let mut found = Vec::new();
        let held = buttons(names)?;
        for i in 0..n {
            self.keys = if i < 9 { held } else { 0 };
            debug::log_calls(4_000_000);
            let r = self.run(1);
            found.extend(debug::take_calls().into_iter().filter(|c| c.args.iter().any(|a| value.contains(a))));
            debug::log_calls(0);
            r?;
        }
        self.keys = 0;
        Ok(found)
    }
}

impl Session {
    /// The function entries of the next `n` frames with an argument that points to `prefix` (bytes).
    pub fn calls_with_text(&mut self, n: u64, prefix: &[u8]) -> Result<Vec<debug::Call>, String> {
        let mut found = Vec::new();
        for _ in 0..n {
            debug::log_calls(4_000_000);
            let r = self.run(1);
            found.extend(debug::take_calls().into_iter().filter(|c| c.args.iter().any(|a| debug::peek(*a, prefix.len()).as_deref() == Some(prefix))));
            debug::log_calls(0);
            r?;
        }
        Ok(found)
    }
}

/// What `autoplay` does: press A after each line, for at most `max_lines` lines.
pub struct Autoplay {
    pub max_lines: usize,
    /// Frames to wait after a line opens before its picture is taken (the text is typed out by then).
    pub delay: u64,
    /// Where the pictures of the dialogue window go, `<scene>_<ID>.png`.
    pub out: Option<PathBuf>,
}

/// Why `autoplay` stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stop {
    /// The count of lines asked for was shown.
    MaxLines,
    /// The scene changed: its last line has been passed.
    SceneEnd,
    /// No new line in ten seconds of A, or the same lines again and again: a puzzle, a walk, a cut scene, or
    /// a wait for an event or a timer that A does not end.
    Stalled,
}

/// The lower screen's rows of the list of requests (y), the page-turner, and the frames the menu needs.
const REQUEST_ROWS: [i32; 3] = [19, 49, 78];
const NEXT_PAGE: (i32, i32) = (55, 75);

impl Session {
    /// From the title to the list of requests (the save must have the requests open), blind by
    /// frames: the menu is the same every run, so is the time it takes.
    fn to_request_list(&mut self) -> Result<(), String> {
        self.run(2400)?;
        for (key, wait) in [("START", 200), ("START", 200), ("START", 150), ("A", 300), ("A", 400), ("A", 300), ("A", 400)] {
            self.press(key, 9)?;
            self.run(wait)?;
        }
        for _ in 0..2 {
            self.tap(200, 80, 9)?;
            self.run(300)?;
        }
        Ok(())
    }

    /// Plays request `n` from its start, through the menu of a save that has played it: the first
    /// scene `Cnn_01` is being read when this returns.
    pub fn goto_request(&mut self, n: u32) -> Result<(), String> {
        self.to_request_list()?;
        // the first touch on "Next" is taken as the list's focus, the rest turn the pages
        let page = n / 3;
        for _ in 0..if page > 0 { page + 1 } else { 0 } {
            self.tap(NEXT_PAGE.0, NEXT_PAGE.1, 9)?;
            self.run(60)?;
        }
        let y = *REQUEST_ROWS.get((n % 3) as usize).ok_or("request row")?;
        self.tap(230, y, 9)?;
        self.run(60)?;
        self.tap(230, y, 9)?;
        let want = format!("C{:02}_01", n);
        for _ in 0..1500 {
            if self.scene.as_deref() == Some(want.as_str()) {
                return Ok(());
            }
            self.run(1)?;
        }
        Err(format!("request {} did not start: the game never read {} (does the save have it open?)", n, want))
    }

    /// Plays the scene `Cnn_yy`: `Cnn_01` is the start of request `nn`; a later one is started by its event
    /// (`goto_event`), `value` being the story step to start it at.
    pub fn goto(&mut self, scene: &str, value: Option<i32>) -> Result<(), String> {
        let bad = || format!("{}: a scene is Cnn_yy, as C04_16", scene);
        let (c, y) = scene.strip_prefix('C').and_then(|r| r.split_once('_')).ok_or_else(bad)?;
        let chapter: u32 = c.parse().map_err(|_| bad())?;
        // a name that is no number (`C04_LIST`, `C04_NEW01`) is an event of the chapter, as the later scenes are
        if y.parse::<u32>() == Ok(1) {
            self.goto_request(chapter)
        } else {
            self.goto_event(scene, value)
        }
    }

    /// The window at the foot of the upper screen as a PNG (taken on a frame drawn for it).
    pub fn shot_dialogue(&mut self, path: &Path) -> Result<(), String> {
        nds::set_draw(true);
        let r = self.run(1);
        let (scale, rgba) = nds::screens_rgba();
        nds::set_draw(false);
        r?;
        let (w, from, to) = (256 * scale, 136 * scale, 192 * scale);
        let rgb: Vec<u8> = rgba[from * w * 4..to * w * 4].chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        if let Some(d) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {}", d.display(), e))?;
        }
        let f = std::fs::File::create(path).map_err(|e| format!("{}: {}", path.display(), e))?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w as u32, (to - from) as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().and_then(|mut e| e.write_image_data(&rgb)).map_err(|e| format!("{}: {}", path.display(), e))
    }

    /// Presses A line after line (the line now shown first), taking each window's picture, until
    /// `o.max_lines` lines, the end of the scene (the lines are those of the scene shown when it began)
    /// or a stall.
    pub fn autoplay(&mut self, o: &Autoplay) -> Result<Stop, String> {
        let scene = self.scene.clone();
        let mut first = self.events.last().filter(|e| e.scene == scene).cloned();
        // lines of this scene seen so far, and the run of shown lines that were seen before
        let mut seen = Repeats::default();
        for n in 0.. {
            if n >= o.max_lines {
                return Ok(Stop::MaxLines);
            }
            let event = match first.take() {
                Some(e) => e,
                None => match self.advance(if n == 0 { 6000 } else { 600 })? {
                    Some(e) => e,
                    // A through a scene's pause (a walk, a camera) can cost the line after it: leave it alone for a while
                    None => {
                        self.run(300)?;
                        match self.advance(600)? {
                            Some(e) => e,
                            None => return Ok(Stop::Stalled),
                        }
                    }
                },
            };
            if event.scene != scene {
                return Ok(Stop::SceneEnd);
            }
            // the same line again and again: the scene waits for something A does not give (an event, a timer)
            if seen.stalled(&event.id) {
                return Ok(Stop::Stalled);
            }
            if let (Some(dir), Some(name), Some(id)) = (&o.out, &event.scene, &event.id) {
                self.run(o.delay)?;
                self.shot_dialogue(&dir.join(format!("{}_{}.png", name, id)))?;
            }
        }
        Ok(Stop::MaxLines)
    }

    /// Plays on as the app's skip does: undisturbed, until the game's script waits for the player's A (a line of
    /// speech, a `WAIT`) or has been idle for 45 frames, for at most `max` frames. The frames it took, and why it
    /// stopped: "waits", "idle" or "limit".
    pub fn skip(&mut self, max: u64) -> Result<(u64, &'static str), String> {
        let (mut idle, mut waits) = (0, 0);
        for i in 0..max {
            self.run(1)?;
            idle = if nds::script_idle() == Some(true) { idle + 1 } else { 0 };
            waits = if nds::script_waits() == Some(true) { waits + 1 } else { 0 };
            if waits >= 2 {
                return Ok((i + 1, "waits"));
            }
            if idle >= 45 {
                return Ok((i + 1, "idle"));
            }
        }
        Ok((max, "limit"))
    }

    /// The state now: scene, line shown, face, and the game's frame.
    pub fn state(&self) -> Option<&Event> {
        self.events.last()
    }
}

/// Text from files put in the game's memory in place of the scenes' own (`Session::use_texts`).
struct Texts {
    /// `Cnn_yy` to the `.s` file of it (one language's folder, or the first found of a scene).
    files: HashMap<String, PathBuf>,
    /// Look at the files' times again each half second of the game's time.
    watch: bool,
    /// The lines of the scenes read so far: the file's stamp and id to text.
    read: HashMap<String, (std::time::SystemTime, HashMap<String, String>)>,
    /// By command: the buffer the text is in now, how many bytes it has, and a hash of what is in it.
    placed: HashMap<u32, (u32, usize, u64)>,
    /// What is left of the block taken from the game's heap: (next, end).
    arena: (u32, u32),
}

/// `.s` files under `dir` by scene name; where a scene has several (languages), the one in a folder `ue` first.
fn find_scripts(dir: &Path) -> HashMap<String, PathBuf> {
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().map_or(false, |x| x == "s") {
                out.push(p);
            }
        }
    }
    let mut all = Vec::new();
    walk(dir, &mut all);
    all.sort();
    let mut files: HashMap<String, PathBuf> = HashMap::new();
    for p in all {
        let Some(name) = p.file_stem().and_then(|n| n.to_str()).map(str::to_string) else { continue };
        if !files.contains_key(&name) || p.components().any(|c| c.as_os_str() == "ue") {
            files.insert(name, p);
        }
    }
    files
}

fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// The game's allocator (Thumb), taking the size in bytes.
const ALLOC: u32 = 0x0202_BFAD;
const ARENA: u32 = 0x1_0000;

impl Session {
    /// From now on the lines of the scenes come from the `.s` files under `dir`: each scene's, when the game
    /// has loaded it, goes over the game's own text (and over it again when a file changes, with `watch`).
    pub fn use_texts(&mut self, dir: &Path, watch: bool) -> Result<(), String> {
        let files = find_scripts(dir);
        if files.is_empty() {
            return Err(format!("{}: no .s files", dir.display()));
        }
        self.texts = Some(Texts { files, watch, read: HashMap::new(), placed: HashMap::new(), arena: (0, 0) });
        Ok(())
    }

    /// A block of `bytes` from the game's heap.
    fn alloc(&mut self, bytes: u32) -> Result<u32, String> {
        let at = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let out = at.clone();
        debug::inject(EVENT_LOOP, move |c| {
            c.r[0] = bytes;
            c.r[13] = c.r[13].wrapping_sub(0x100) & !7;
            c.r[14] = EVENT_LOOP;
            crate::call(c, ALLOC);
            out.set(c.r[0]);
        });
        for _ in 0..300 {
            if at.get() != 0 {
                return Ok(at.get());
            }
            self.run(1)?;
        }
        Err("the game did not reach its event loop to give memory".into())
    }

    /// Puts the replacement lines of the scene now played over the game's.
    fn apply_texts(&mut self) -> Result<(), String> {
        let Some(mut t) = self.texts.take() else { return Ok(()) };
        let r = self.apply_with(&mut t);
        self.texts = Some(t);
        r
    }

    fn apply_with(&mut self, t: &mut Texts) -> Result<(), String> {
        let Some(scene) = self.scene.clone() else { return Ok(()) };
        let Some(path) = t.files.get(&scene).cloned() else { return Ok(()) };
        let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).map_err(|e| format!("{}: {}", path.display(), e))?;
        if t.read.get(&scene).map_or(true, |(s, _)| t.watch && *s != stamp) {
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
            let map = crate::script::lines(&crate::script::utf16le(&bytes)).into_iter().map(|l| (l.id, l.text)).collect();
            t.read.insert(scene.clone(), (stamp, map));
        }
        let lines = &t.read[&scene].1;
        for (cmd, id, ptr) in self.tracker.loaded() {
            let Some(text) = lines.get(&format!("{:05}", id)) else { continue };
            let h = hash(text);
            let mut units: Vec<u16> = text.encode_utf16().collect();
            units.push(0);
            let need = units.len() * 2;
            let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
            match t.placed.get(&cmd).copied() {
                Some((buf, _, done)) if buf == ptr && done == h => {}
                // our own buffer with room: the text goes in where the game reads it
                Some((buf, cap, _)) if buf == ptr && need <= cap => {
                    debug::poke(buf, &bytes);
                    t.placed.insert(cmd, (buf, cap, h));
                }
                _ => {
                    if t.arena.0 + need as u32 > t.arena.1 {
                        let block = self.alloc(ARENA)?;
                        t.arena = (block, block + ARENA);
                    }
                    let cap = (need + 63) & !63;
                    let buf = t.arena.0;
                    t.arena.0 += cap as u32;
                    debug::poke(buf, &bytes);
                    debug::poke(cmd + 8, &buf.to_le_bytes());
                    Tracker::rebind_shown(ptr, buf);
                    t.placed.insert(cmd, (buf, cap, h));
                }
            }
        }
        Ok(())
    }
}

/// The game's own handlers of script commands, run from the host at a safe point (the top of its
/// per-frame event loop) with a command made for them: what the scripts' `SETVALUE` and `CHANGEMAP` do.
const EVENT_LOOP: u32 = 0x0202_3745;
const COMMAND_SLOT: u32 = 0x0207_596C + 0x1C;
const SETVALUE: u32 = 0x0204_859D;
const CHANGEMAP: u32 = 0x0204_8669;
const VALUES: u32 = 0x0207_42EC;
/// The pointer to the world (its first word the hero, whose position is at +0x18).
const WORLD: u32 = 0x0206_B45C + 4;
/// In the world: the number of events and the table of their records (the loop `EVENT_LOOP` goes over it).
const EVENT_COUNT: u32 = 0x1E80C;
const EVENT_TABLE: u32 = 0x1E810;

impl Session {
    /// Runs the game's handler `handler` on a command whose parameters (from the third word on) are `params`.
    fn script_command(&mut self, handler: u32, params: Vec<i32>) -> Result<(), String> {
        let done = std::rc::Rc::new(std::cell::Cell::new(false));
        let flag = done.clone();
        debug::inject(EVENT_LOOP, move |c| {
            let sp = c.r[13].wrapping_sub(0x100) & !7;
            let mut block = [0u8; 0x40];
            for (i, p) in params.iter().enumerate() {
                block[8 + 4 * i..12 + 4 * i].copy_from_slice(&p.to_le_bytes());
            }
            let old = debug::peek(COMMAND_SLOT, 4).unwrap_or_default();
            debug::poke(sp, &block);
            debug::poke(COMMAND_SLOT, &sp.to_le_bytes());
            c.r[13] = sp;
            c.r[14] = EVENT_LOOP;
            crate::call(c, handler);
            debug::poke(COMMAND_SLOT, &old);
            flag.set(true);
        });
        for _ in 0..300 {
            if done.get() {
                return Ok(());
            }
            self.run(1)?;
        }
        Err("the game did not reach its event loop".into())
    }

    /// The story value `index` (`SETVALUE index value`; 0 is the chapter's step).
    pub fn set_value(&mut self, index: i32, value: i32) -> Result<(), String> {
        self.script_command(SETVALUE, vec![index, value])
    }

    /// The story value `index` now.
    pub fn value(&self, index: u32) -> Option<i32> {
        debug::peek(VALUES + 4 * index, 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Puts the hero at `pos` (1/4096 units) of the map he is on; false if the map has no hero yet.
    pub fn teleport(&mut self, pos: [i32; 3]) -> bool {
        let word = |a: u32| debug::peek(a, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let Some(hero) = word(WORLD).and_then(|w| word(w)).filter(|h| *h != 0) else { return false };
        let bytes: Vec<u8> = pos.iter().flat_map(|p| p.to_le_bytes()).collect();
        debug::poke(hero + 0x18, &bytes)
    }

    /// Moves the hero to map `map` at `pos` (1/4096 units), as a script's `CHANGEMAP` does.
    pub fn change_map(&mut self, map: i32, pos: [i32; 3]) -> Result<(), String> {
        self.script_command(CHANGEMAP, vec![map, pos[0], pos[1], pos[2], 0])
    }

    /// A file of the ROM by its path in the NitroFS.
    fn file(&self, path: &str) -> Result<Vec<u8>, String> {
        let (at, end) = self.fs.place(path).ok_or_else(|| format!("{}: not in the ROM", path))?;
        let len = end - at;
        let mut rom = std::fs::File::open(&self.rom).map_err(|e| format!("{}: {}", self.rom.display(), e))?;
        let mut bytes = vec![0u8; len as usize];
        std::io::Seek::seek(&mut rom, std::io::SeekFrom::Start(at as u64)).and_then(|_| std::io::Read::read_exact(&mut rom, &mut bytes)).map_err(|e| format!("{}: {}", path, e))?;
        Ok(bytes)
    }

    /// The map the hero is on.
    /// The scene whose script the game read last.
    pub fn scene(&self) -> Option<&str> {
        self.scene.as_deref()
    }

    pub fn map(&self) -> Option<i32> {
        let word = |a: u32| debug::peek(a, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let hero = word(WORLD).and_then(|w| word(w)).filter(|h| *h != 0)?;
        word(hero + 0xC).map(|m| m as i32)
    }

    /// Takes the door `door` (an event `Ann_yy` whose script changes the map): the hero to its place, A, and
    /// the wait for the map.
    fn take_door(&mut self, door: &Door) -> Result<(), String> {
        if !self.teleport(door.event.pos) {
            return Err("no hero to move".into());
        }
        self.run(20)?;
        for i in 0..600 {
            if self.map() == Some(door.target) {
                self.run(240)?;
                return Ok(());
            }
            if i % 120 == 30 {
                self.press("A", 9)?;
            } else {
                self.run(1)?;
            }
        }
        Err(format!("the door {} (map {}) did not lead to map {}", door.event.scene, door.event.map, door.target))
    }

    /// Plays to the scene `scene` of a chapter: the chapter from its start to the first free walk, then the
    /// story value its event wants (from the chapter's `.EVN`), the doors from map to map as the game has them
    /// (each by its own event), and the place of the event, where the game starts the scene itself.
    pub fn goto_event(&mut self, scene: &str, value: Option<i32>) -> Result<(), String> {
        self.goto_event_from(scene, value, false)
    }

    /// As `goto_event` for the next scene of a run: from where the last one left the game, not from the start of
    /// the chapter. The scene before is played out as far as it goes first (`settle`); false if the game does not
    /// come to a free walk, and then only a fresh start will do.
    pub fn goto_next(&mut self, scene: &str, value: Option<i32>) -> Result<bool, String> {
        // a scene no event starts does not wait for the game to settle
        self.event_of(scene)?;
        if !self.settle()? {
            return Ok(false);
        }
        self.goto_event_from(scene, value, true).map(|_| true)
    }

    /// Plays on, A now and then, until no script runs and the hero is on a map (a free walk), for at most 40 s.
    pub fn settle(&mut self) -> Result<bool, String> {
        let mut idle = 0;
        for i in 0..2400 {
            idle = if nds::script_idle() == Some(true) && self.map().is_some() { idle + 1 } else { 0 };
            if idle >= 60 {
                return Ok(true);
            }
            if i % 60 == 30 {
                self.press("A", 5)?;
            } else {
                self.run(1)?;
            }
        }
        Ok(false)
    }

    /// The chapter's number, its events (`.EVN`), and the one that starts `scene`.
    fn event_of(&self, scene: &str) -> Result<(u32, Vec<crate::evn::Event>, crate::evn::Event), String> {
        let chapter: u32 = scene.get(1..3).and_then(|c| c.parse().ok()).ok_or_else(|| format!("{}: a scene is Cnn_yy", scene))?;
        let events = crate::evn::parse(&self.file(&format!("Chapter/Pack/C{:02}/C{:02}.EVN", chapter, chapter))?);
        let event = events.iter().find(|e| e.scene == scene).cloned().ok_or_else(|| format!("{}: no event starts it in chapter {}", scene, chapter))?;
        Ok((chapter, events, event))
    }

    fn goto_event_from(&mut self, scene: &str, value: Option<i32>, carry_on: bool) -> Result<(), String> {
        let (chapter, events, event) = self.event_of(scene)?;
        let mut doors = Vec::new();
        for e in events.iter().filter(|e| e.scene.starts_with('A')) {
            // a name the file holds that is no scene of this chapter is not a door
            let Ok(z) = self.file(&format!("Chapter/Pack/C{:02}/{}.txt.z", chapter, e.scene)) else { continue };
            let target = crate::script::lz10(&z).and_then(|t| crate::script::map_target(&crate::script::utf16le(&t)));
            doors.extend(target.map(|target| Door { event: e.clone(), target }));
        }
        if !carry_on {
            self.quiet = self.quiet_opening;
            let played = self.goto_request(chapter).and_then(|_| self.autoplay(&Autoplay { max_lines: 1000, delay: 0, out: None }));
            self.quiet = false;
            // the opening plays by A; the first stall is the first free walk
            let stop = played?;
            if stop == Stop::MaxLines {
                return Err(format!("the opening of chapter {} did not end in 1000 lines", chapter));
            }
            // the opening runs on into the scene (C04_01 into C04_02): it is there already
            if self.scene.as_deref() == Some(scene) {
                return Ok(());
            }
        }
        self.set_value(0, value.unwrap_or(event.value_min))?;
        let here = self.map().ok_or("the hero is not on a map: the opening has not ended")?;
        for door in route(&doors, here, event.map).ok_or_else(|| format!("no way from map {} to map {} by the {} doors of chapter {} ({:?})", here, event.map, doors.len(), chapter, doors.iter().map(|d| (d.event.map, d.target)).collect::<Vec<_>>()))? {
            self.take_door(&door)?;
        }
        let silenced = self.silence_rivals(scene);
        if !self.teleport(event.pos) {
            return Err("no hero to move".into());
        }
        for i in 0..900 {
            if self.scene.as_deref() == Some(scene) {
                return Ok(());
            }
            // a scene started by a talk wants A while the hero stands there
            if i % 90 == 60 {
                self.press("A", 9)?;
            } else {
                self.run(1)?;
            }
        }
        Err(format!("the hero stands at the place of {} (map {}) but the game did not start it: more of the story is needed (--set value0=N), or another place (the other events of the chapter were put out: {})", scene, event.map, silenced))
    }

    /// Puts out the scene events of the chapter other than `scene` (the first byte of the name in the game's
    /// event table becomes 0, so its loop passes them by): at the same place and story step, the game starts
    /// the first of them that holds (C04_06 before C04_08, as both are met at step 30). The doors stay. The
    /// number put out.
    fn silence_rivals(&mut self, scene: &str) -> usize {
        let word = |a: u32| debug::peek(a, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let Some(world) = word(WORLD) else { return 0 };
        let count = word(world + EVENT_COUNT).unwrap_or(0).min(512);
        let table: Vec<u32> = (0..count).filter_map(|i| word(world + EVENT_TABLE + 4 * i)).collect();
        // the names put out before come back, those whose record is still in the table
        for (at, byte) in std::mem::take(&mut self.silenced) {
            if table.contains(&at) && debug::peek(at, 1) == Some(vec![0]) {
                debug::poke(at, &[byte]);
            }
        }
        let mut out = 0;
        for event in table.into_iter().filter(|e| *e != 0) {
            let Some(name) = debug::peek(event, 8) else { continue };
            if name[0] != 0 && !name.starts_with(scene.as_bytes()) && !name.starts_with(b"A") && debug::poke(event, &[0]) {
                self.silenced.push((event, name[0]));
                out += 1;
            }
        }
        out
    }
}

/// A door: the event that is on a map, and the map its script leads to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Door {
    pub event: crate::evn::Event,
    pub target: i32,
}

/// The doors to take from map `from` to map `to`, the fewest (breadth first); empty if they are the same map.
pub fn route(doors: &[Door], from: i32, to: i32) -> Option<Vec<Door>> {
    let mut came: HashMap<i32, Option<usize>> = HashMap::from([(from, None)]);
    let mut queue = std::collections::VecDeque::from([from]);
    while let Some(map) = queue.pop_front() {
        if map == to {
            let mut path = Vec::new();
            let mut at = map;
            while let Some(Some(i)) = came.get(&at) {
                path.push(doors[*i].clone());
                at = doors[*i].event.map;
            }
            path.reverse();
            return Some(path);
        }
        for (i, d) in doors.iter().enumerate().filter(|(_, d)| d.event.map == map) {
            if !came.contains_key(&d.target) {
                came.insert(d.target, Some(i));
                queue.push_back(d.target);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evn::Event;

    fn door(scene: &str, from: i32, target: i32) -> Door {
        Door { event: Event { scene: scene.into(), map: from, pos: [0; 3], value_min: 0, flags: 0, low: 0, high: 0 }, target }
    }

    #[test]
    fn lines_shown_again_and_again_are_a_stall() {
        let mut r = Repeats::default();
        let id = |n: u32| Some(format!("{:05}", n));
        // two lines taking turns: new ones stay at two, the sixth repeat stops it
        let stops: Vec<_> = (0..10).map(|i| r.stalled(&id(3001 + i % 2))).collect();
        assert_eq!(stops.iter().position(|s| *s), Some(7));
        // lines that go on are no stall, whatever came before
        let mut r = Repeats::default();
        assert!((0..50).all(|i| !r.stalled(&id(i))));
    }

    #[test]
    fn the_route_takes_the_fewest_doors() {
        let doors = [door("A01", 1, 2), door("A02", 2, 3), door("A03", 1, 3), door("A04", 3, 4)];
        let r: Vec<_> = route(&doors, 1, 4).unwrap().into_iter().map(|d| d.event.scene).collect();
        assert_eq!(r, ["A03", "A04"]);
        assert!(route(&doors, 1, 1).unwrap().is_empty());
        assert!(route(&doors, 4, 1).is_none());
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        nds::finish();
    }
}
