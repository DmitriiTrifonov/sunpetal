//! Tools for proofreading the translation (docs/DEBUG-TOOLS-PLAN.md): runs the recompiled game
//! and reports on it as JSON lines on stdout and files. Needs `--features debug-tools`.
//!
//!     sunpetal-debug ROM.nds [--save FILE] [--repl]
//!     sunpetal-debug frame REC N [--out DIR]     what a recording holds of DS frame N (no ROM needed)
//!
//! `--repl` reads commands from stdin, one per line, and answers each with a line of JSON
//! (`{"ok":true,...}` or `{"error":"..."}`):
//!
//!     run N | press KEYS [FRAMES] | tap X Y [FRAMES] | shot FILE | shotwide FILE (with --wide)
//!     dump2d DIR | state | log FILE|off | advance [FRAMES] | walk N [FRAMES] | shown | peek ADDR[:LEN] | find TEXT | findhex HEX | calls FRAMES FILE [KEYS] | callsarg FRAMES LO[-HI] [KEYS] | quit
use std::io::BufRead;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use serde_json::{json, Value};
use sunpetal_runtime::dialogue::Tracker;
use sunpetal_runtime::nds::debug;
use sunpetal_runtime::session::{Autoplay, Session};

#[derive(Parser)]
#[command(about = "Runs the game and reports on it (JSON lines on stdout)")]
struct Args {
    rom: PathBuf,
    /// The save file (64 KiB EEPROM); a new one is made if it is not there.
    #[arg(long)]
    save: Option<PathBuf>,
    /// A folder of files that replace the ROM's by their NitroFS paths (the translation).
    #[arg(long)]
    overlay: Option<PathBuf>,
    /// A data file (`sunpetal.data`); with `--lang` its translation goes in.
    #[arg(long)]
    data: Option<PathBuf>,
    /// The language of the translation in the data file (`ru`).
    #[arg(long)]
    lang: Option<String>,
    /// Commands from stdin, one per line.
    #[arg(long)]
    repl: bool,
    /// Write one JSON line per dialogue window opened to this file.
    #[arg(long, value_name = "FILE")]
    dialogue_log: Option<PathBuf>,
    /// Play to this scene (Cnn_yy) through the menu of requests first; several, as C04_01,C04_03, one after the
    /// other in this process (each is a line of JSON with its status).
    #[arg(long, value_name = "Cnn_yy[,Cnn_yy...]")]
    goto: Option<String>,
    /// The scenes to play one after the other, one a line of FILE (`#` starts a note), as a list for --goto.
    #[arg(long, value_name = "FILE", conflicts_with_all = ["goto", "chapter"])]
    scenes: Option<PathBuf>,
    /// Play request N from its start (the same as --goto CNN_01).
    #[arg(long, value_name = "N", conflicts_with = "goto")]
    chapter: Option<u32>,
    /// A story value, valueN=V (`SETVALUE N V`); value0 is the chapter's step.
    #[arg(long, value_name = "NAME=VALUE")]
    set: Vec<String>,
    /// Press A after each line to the end of the scene.
    #[arg(long)]
    autoplay: bool,
    /// Autoplay: stop after this many lines.
    #[arg(long, default_value_t = 200)]
    max_lines: usize,
    /// Autoplay: frames to wait after a line opens before its picture is taken.
    #[arg(long, default_value_t = 120)]
    frame_delay: u64,
    /// Autoplay: a picture of the dialogue window per line goes to DIR/<scene>_<ID>.png.
    #[arg(long, value_name = "DIR")]
    out: Option<PathBuf>,
    /// Autoplay: both screens per line go to DIR/<scene>_<ID>.png (the input of `sunpetal-screen-report`).
    #[arg(long, value_name = "DIR")]
    screens_dir: Option<PathBuf>,
    /// Both screens as a PNG at the end.
    #[arg(long, value_name = "FILE")]
    shot: Option<PathBuf>,
    /// Widen the 3D view by this many DS pixels each side; `--shot` then also writes FILE with `_wide` before the
    /// extension (the upper screen alone, 256 + 2N wide).
    #[arg(long, value_name = "PIXELS")]
    wide: Option<usize>,
    /// A 2D pack (a folder of pictures or a data file with a 2D part) that replaces sprites and background layers.
    #[arg(long, value_name = "DIR|FILE")]
    pack_2d: Option<PathBuf>,
    /// The variant of a 2D pack group to draw, `group=variant` (repeatable; `original` is the game's own sprites).
    #[arg(long, value_name = "GROUP=VARIANT")]
    pack_variant: Vec<String>,
    /// The 3D's internal resolution (1-8 times the DS's) for `--shot`.
    #[arg(long, value_name = "N")]
    scale: Option<usize>,
    /// The 3D effects, as `sunpetal-headless` takes them (0-1; 0 off).
    #[arg(long)]
    outline: Option<f32>,
    #[arg(long)]
    outline_width: Option<f32>,
    #[arg(long)]
    shadow: Option<f32>,
    #[arg(long)]
    shadow_radius: Option<f32>,
    /// Supersampling of the 3D edges (0, 2 or 4).
    #[arg(long, value_name = "0|2|4")]
    msaa: Option<u8>,
    /// nearest | bilinear.
    #[arg(long)]
    texture_filter: Option<String>,
    /// The engine that draws the 3D: software (default) or gpu (needs the `gpu` feature).
    #[arg(long, value_name = "software|gpu")]
    renderer: Option<String>,
    /// Print the state (last line, frame, map, story value 0, why autoplay stopped) as a JSON line at the end.
    #[arg(long)]
    state: bool,
    /// The lines of the scenes come from the `.s` files under this folder (one language's), put over the
    /// game's own as each scene loads.
    #[arg(long, value_name = "DIR")]
    text_dir: Option<PathBuf>,
    /// With --text-dir: put a file's text in again when it changes (checked twice a second of game time).
    #[arg(long)]
    watch: bool,
    /// Add to the dialogue log instead of making it anew (the run of the rest of a list that the program
    /// starts itself, when the game has to be booted again).
    #[arg(long, hide = true)]
    append_log: bool,
}

fn number<T: std::str::FromStr>(w: Option<&&str>, name: &str, default: Option<T>) -> Result<T, String> {
    match w {
        Some(s) => s.parse().map_err(|_| format!("{}: not a number: {}", name, s)),
        None => default.ok_or_else(|| format!("{} is missing", name)),
    }
}

fn address(s: &str) -> Result<u32, String> {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|_| format!("{}: bad address", s))
}

/// One command; `Ok(None)` ends the session.
fn command(s: &mut Session, tracker: &Tracker, line: &str) -> Result<Option<Value>, String> {
    let (name, rest) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
    let w: Vec<&str> = rest.split_whitespace().collect();
    let done = |extra: Value| -> Result<Option<Value>, String> {
        let mut v = json!({"ok": true, "vblank": nds_vblank()});
        v.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
        Ok(Some(v))
    };
    match name {
        "setvalue" => {
            s.set_value(number(w.first(), "INDEX", None)?, number(w.get(1), "VALUE", None)?)?;
            done(json!({}))
        }
        "value" => done(json!({"value": s.value(number(w.first(), "INDEX", Some(0))?)})),
        "teleport" => {
            let ok = s.teleport([number(w.first(), "X", None)?, number(w.get(1), "Y", None)?, number(w.get(2), "Z", None)?]);
            done(json!({"moved": ok}))
        }
        "changemap" => {
            s.change_map(number(w.first(), "MAP", None)?, [number(w.get(1), "X", None)?, number(w.get(2), "Y", None)?, number(w.get(3), "Z", None)?])?;
            done(json!({}))
        }
        "scene" => done(json!({"scene": s.scene()})),
        "scriptstate" => done(
            json!({"loaded": sunpetal_runtime::nds::script_loaded(), "idle": sunpetal_runtime::nds::script_idle(), "waits": sunpetal_runtime::nds::script_waits(), "mark": sunpetal_runtime::nds::script_mark()}),
        ),
        "skip" => {
            let (frames, why) = s.skip(number(w.first(), "FRAMES", Some(18000))?)?;
            done(json!({"frames": frames, "stopped": why}))
        }
        "goto" => {
            s.goto(w.first().ok_or("goto Cnn_yy")?, None)?;
            done(json!({}))
        }
        "request" => {
            s.goto_request(number(w.first(), "N", None)?)?;
            done(json!({}))
        }
        "run" => {
            s.run(number(w.first(), "N", None)?)?;
            done(json!({}))
        }
        "press" => {
            s.press(w.first().ok_or("press KEYS [FRAMES]")?, number(w.get(1), "FRAMES", Some(9))?)?;
            done(json!({}))
        }
        "tap" => {
            s.tap(number(w.first(), "X", None)?, number(w.get(1), "Y", None)?, number(w.get(2), "FRAMES", Some(9))?)?;
            done(json!({}))
        }
        "drag" => {
            let c = |i, n: &str| number::<i32>(w.get(i), n, None);
            s.drag((c(0, "X0")?, c(1, "Y0")?), (c(2, "X1")?, c(3, "Y1")?), number(w.get(4), "FRAMES", Some(20))?)?;
            done(json!({}))
        }
        "shot" => {
            s.shot(std::path::Path::new(w.first().ok_or("shot FILE")?))?;
            done(json!({}))
        }
        "dump2d" => {
            let n = sunpetal_runtime::nds::dump_2d(std::path::Path::new(w.first().ok_or("dump2d DIR")?));
            done(json!({"new": n}))
        }
        "traceall" => {
            debug::trace_everything(true);
            done(json!({}))
        }
        "cliprep" => {
            let top: Vec<Value> =
                debug::clip_report().into_iter().take(20).map(|((k, lr), n)| json!({"fn": format!("{:08X}", k), "lr": format!("{:08X}", lr), "reads": n})).collect();
            let callers: Vec<Value> = debug::clip_callers().into_iter().take(20).map(|(lr, n)| json!({"lr": format!("{:08X}", lr), "calls": n})).collect();
            done(json!({"top": top, "callers": callers}))
        }
        "shotwide" => {
            let wide = sunpetal_runtime::nds::screenshot_wide(std::path::Path::new(w.first().ok_or("shotwide FILE")?))?;
            done(json!({"wide": wide}))
        }
        "peek" => {
            let (a, n) = w.first().ok_or("peek ADDR[:LEN]")?.split_once(':').unwrap_or((w[0], "64"));
            let addr = address(a)?;
            let len = n.parse().map_err(|_| format!("{}: bad length", n))?;
            let bytes = debug::peek(addr, len).ok_or_else(|| format!("{:08X}: not memory", addr))?;
            done(json!({"addr": format!("{:08X}", addr), "hex": bytes.iter().map(|b| format!("{:02X}", b)).collect::<String>()}))
        }
        "watchw" => {
            // watchw ADDR LEN [CAP] [r]: record the ARM9's stores (and with `r` its loads) into [ADDR, ADDR+LEN) until `unwatchw`; `writes` reads them
            let addr = address(w.first().ok_or("watchw ADDR LEN [CAP]")?)?;
            debug::watch_writes(addr, number(w.get(1), "LEN", Some(4))?, number(w.get(2), "CAP", Some(100_000))?, w.get(3) == Some(&"r"));
            done(json!({}))
        }
        "unwatchw" => {
            debug::stop_writes();
            done(json!({}))
        }
        "writes" => {
            // the stores since the last call, grouped by writer (function entered last, its caller, link register)
            let mut by: std::collections::BTreeMap<(bool, u32, u32, u32), (u64, Vec<String>)> = Default::default();
            for e in debug::take_writes() {
                let g = by.entry((e.read, e.func, e.func_lr, e.lr)).or_default();
                g.0 += 1;
                if g.1.len() < 6 {
                    g.1.push(format!("{:08X}:{}={:08X}@{}", e.addr, e.size, e.value, e.vblank));
                }
            }
            let rows: Vec<Value> = by
                .iter()
                .map(|((rd, f, flr, lr), (n, s))| json!({"read": rd, "fn": format!("{:08X}", f), "fn_lr": format!("{:08X}", flr), "lr": format!("{:08X}", lr), "stores": n, "first": s}))
                .collect();
            done(json!({"writers": rows}))
        }
        "poke" => {
            let addr = address(w.first().ok_or("poke ADDR VALUE")?)?;
            let value: u32 = number(w.get(1), "VALUE", None)?;
            if !debug::poke(addr, &value.to_le_bytes()) {
                return Err(format!("{:08X}: not memory", addr));
            }
            done(json!({}))
        }
        "dump" => {
            let (addr, len) = (address(w.first().ok_or("dump ADDR LEN FILE")?)?, number(w.get(1), "LEN", None)?);
            let bytes = debug::peek(addr, len).ok_or_else(|| format!("{:08X}: not memory", addr))?;
            let path = w.get(2).ok_or("dump ADDR LEN FILE")?;
            std::fs::write(path, bytes).map_err(|e| format!("{}: {}", path, e))?;
            done(json!({}))
        }
        "find" => {
            let at: Vec<String> = debug::find(&debug::utf16(rest)).iter().map(|a| format!("{:08X}", a)).collect();
            done(json!({"at": at}))
        }
        "log" => {
            s.log_to(w.first().filter(|p| **p != "off").map(std::path::Path::new), false)?;
            done(json!({}))
        }
        "advance" => {
            let e = s.advance(number(w.first(), "FRAMES", Some(600))?)?;
            done(json!({"line": e}))
        }
        "walk" => {
            let (n, max): (usize, u64) = (number(w.first(), "N", None)?, number(w.get(1), "FRAMES", Some(600))?);
            let mut seen = 0;
            while seen < n && s.advance(max)?.is_some() {
                seen += 1;
            }
            done(json!({"lines": seen}))
        }
        "state" => {
            // the 2D engines' registers and OAM now, decoded as `frame` does for a recording
            let v: Value = serde_json::from_str(&format!("{{{}}}", sunpetal_runtime::nds::frame_state())).map_err(|e| e.to_string())?;
            done(json!({"a": frame::engine(&v["a"]), "b": frame::engine(&v["b"])}))
        }
        "shown" => {
            let v = tracker.shown().map(|l| json!({"ptr": format!("{:08X}", l.ptr), "id": l.id, "text": l.text}));
            done(json!({"shown": v}))
        }
        "slots" => {
            let (from, n): (u32, u32) = (number(w.first(), "FROM", None)?, number(w.get(1), "N", Some(8))?);
            let rows: Vec<Value> = (from..from + n)
                .map(|i| {
                    let at = debug::peek(0x0207_4BEC + 4 * i, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0);
                    let hex = debug::peek(at, 0x40).map(|b| b.chunks(4).map(|c| format!("{:08X}", u32::from_le_bytes([c[0], c[1], c[2], c[3]]))).collect::<Vec<_>>());
                    json!({"slot": i, "at": format!("{:08X}", at), "words": hex})
                })
                .collect();
            done(json!({"slots": rows}))
        }
        "findhex" => {
            let bytes: Vec<u8> =
                (0..rest.len() / 2).map(|i| u8::from_str_radix(&rest[2 * i..2 * i + 2], 16).map_err(|_| format!("{}: not hex", rest))).collect::<Result<_, _>>()?;
            let at: Vec<String> = debug::find(&bytes).iter().map(|a| format!("{:08X}", a)).collect();
            done(json!({"at": at}))
        }
        "callsarg" => {
            let (lo, hi) = w.get(1).ok_or("callsarg FRAMES LO[-HI] [KEYS]")?.split_once('-').unwrap_or((w[1], w[1]));
            let value = number(Some(&lo), "LO", None)?..=number(Some(&hi), "HI", None)?;
            let found = s.calls_with(number(w.first(), "FRAMES", None)?, value, w.get(2).copied().unwrap_or(""))?;
            let rows: Vec<Value> = found.iter().map(|c| json!({"vblank": c.vblank, "cpu": c.cpu, "key": format!("{:08X}", c.key), "lr": format!("{:08X}", c.lr), "args": c.args.iter().map(|a| format!("{:08X}", a)).collect::<Vec<_>>()})).collect();
            done(json!({"calls": rows}))
        }
        "callsstr" => {
            let found = s.calls_with_text(number(w.first(), "FRAMES", None)?, w.get(1).ok_or("callsstr FRAMES PREFIX")?.as_bytes())?;
            let rows: Vec<Value> = found.iter().map(|c| json!({"vblank": c.vblank, "cpu": c.cpu, "key": format!("{:08X}", c.key), "lr": format!("{:08X}", c.lr), "args": c.args.iter().map(|a| format!("{:08X}", a)).collect::<Vec<_>>()})).collect();
            done(json!({"calls": rows}))
        }
        "calls" => {
            let rows = s.calls(number(w.first(), "FRAMES", None)?, 4_000_000, w.get(2).copied().unwrap_or(""))?;
            let path = w.get(1).ok_or("calls FRAMES FILE")?;
            let text: String = rows
                .iter()
                .map(|(n, c)| format!("{}\t{:08X}\t{}\t{}\t{:08X}\t{:08X} {:08X} {:08X} {:08X}\n", c.cpu, c.key, n, c.vblank, c.lr, c.args[0], c.args[1], c.args[2], c.args[3]))
                .collect();
            std::fs::write(path, text).map_err(|e| format!("{}: {}", path, e))?;
            done(json!({"functions": rows.len()}))
        }
        "callstrace" => {
            // callstrace FRAMES OUT KEY[,KEY...]: every ARM9 entry into these functions (hex, bit 0 for Thumb), in order (K-189)
            let keys = w
                .get(2)
                .ok_or("callstrace FRAMES OUT KEY[,KEY...]")?
                .split(',')
                .map(|k| u32::from_str_radix(k.trim_start_matches("0x"), 16).map_err(|_| format!("bad key {}", k)))
                .collect::<Result<_, _>>()?;
            let found = s.calls_trace(number(w.first(), "FRAMES", None)?, &keys)?;
            let path = w.get(1).ok_or("callstrace FRAMES OUT KEY")?;
            let text: String =
                found.iter().map(|c| format!("{}\t{:08X}\t{:08X}\t{:08X} {:08X} {:08X} {:08X}\n", c.vblank, c.key, c.lr, c.args[0], c.args[1], c.args[2], c.args[3])).collect();
            std::fs::write(path, text).map_err(|e| format!("{}: {}", path, e))?;
            done(json!({"entries": found.len()}))
        }
        "scriptseq" => {
            // scriptseq FRAMES OUT [DISPATCH_CSV]: the script step's handlers entered, in order (K-162)
            let csv = w.get(2).copied().unwrap_or("docs/native-script-dispatch.csv");
            let text = std::fs::read_to_string(csv).map_err(|e| format!("{}: {}", csv, e))?;
            let mut by_key = std::collections::HashMap::new();
            for l in text.lines().skip(1) {
                let f: Vec<&str> = l.split(',').collect();
                if let (Some(code), Some(h)) = (f.first().and_then(|c| c.parse::<u32>().ok()), f.get(3).and_then(|h| u32::from_str_radix(h, 16).ok())) {
                    by_key.insert(h | 1, (code, f.get(1).copied().unwrap_or("").to_string()));
                }
            }
            let keys = by_key.keys().copied().collect();
            let seq = s.calls_in_order(number(w.first(), "FRAMES", None)?, &keys)?;
            let path = w.get(1).ok_or("scriptseq FRAMES OUT [CSV]")?;
            let out: String = seq.iter().map(|(v, k, a)| format!("{}\t{}\t{}\t{:08X}\n", v, by_key[k].0, by_key[k].1, a)).collect();
            std::fs::write(path, out).map_err(|e| format!("{}: {}", path, e))?;
            done(json!({"entries": seq.len()}))
        }
        "dialogmodel" => {
            // dialogmodel FRAMES [CSV [TRACE]]: the native dialogue model beside the game, words compared each frame (K-177)
            let csv = w.get(1).copied().unwrap_or("docs/native-script-dispatch.csv");
            let by_key = sunpetal_runtime::scriptvm::handlers(&std::fs::read_to_string(csv).map_err(|e| format!("{}: {}", csv, e))?);
            let trace = w.get(2).map(std::path::Path::new);
            let r = s.dialog_model_check(number(w.first(), "FRAMES", None)?, &by_key, trace)?;
            done(serde_json::to_value(&r).map_err(|e| e.to_string())?)
        }
        "dialogref" => {
            // dialogref FRAMES OUT: per frame the scene, the hash of both screens and the dialogue state words (K-176)
            let path = w.get(1).ok_or("dialogref FRAMES OUT")?;
            let text = s.dialog_reference(number(w.first(), "FRAMES", None)?)?;
            std::fs::write(path, &text).map_err(|e| format!("{}: {}", path, e))?;
            done(json!({"frames": text.lines().count(), "columns": sunpetal_runtime::session::DIALOG_STATE.iter().map(|d| d.0).collect::<Vec<_>>()}))
        }
        "scriptcheck" => {
            // scriptcheck FRAMES [DISPATCH_CSV]: the native interpreter follows the game's script (K-162)
            let csv = w.get(1).copied().unwrap_or("docs/native-script-dispatch.csv");
            let text = std::fs::read_to_string(csv).map_err(|e| format!("{}: {}", csv, e))?;
            let by_key = sunpetal_runtime::scriptvm::handlers(&text);
            let r = s.check_script(number(w.first(), "FRAMES", None)?, &by_key)?;
            done(json!({"scenes": r.scenes, "agreed": r.agreed, "total": r.total, "ended": r.ended, "mismatch": r.mismatch.map(|(v, e)| format!("vblank {}: {}", v, e))}))
        }
        "scriptlive" => {
            // scriptlive FRAMES: the observer the app runs (nds::script_observer_*), A pressed on its own (K-162)
            let n = number(w.first(), "FRAMES", None)?;
            if !sunpetal_runtime::nds::script_observer_start() {
                return Err("the observer needs the European dump".into());
            }
            s.observe_live(n)?;
            done(json!({"observer": sunpetal_runtime::nds::script_observer_summary()}))
        }
        "glyphs" => {
            // the canvases the game draws glyphs into (`020338E0`, first argument), summed from the first call of this command
            thread_local! { static SEEN: std::rc::Rc<std::cell::RefCell<std::collections::BTreeMap<u32, u64>>> = Default::default(); static ON: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
            if !ON.with(|o| o.replace(true)) {
                let sink = SEEN.with(|s| s.clone());
                debug::watch(9, 0x0203_38E0, move |c| *sink.borrow_mut().entry(c.r[0]).or_default() += 1);
            }
            s.run(number(w.first(), "FRAMES", Some(0))?)?;
            let rows: Vec<Value> = SEEN.with(|m| m.borrow().iter().map(|(k, n)| json!({"canvas": format!("{:08X}", k), "glyphs": n})).collect());
            done(json!({"canvases": rows}))
        }
        "quit" => Ok(None),
        _ => Err(format!("{}: no such command", name)),
    }
}

fn nds_vblank() -> u64 {
    sunpetal_runtime::nds::vblanks()
}

fn run(a: Args) -> Result<(), String> {
    if a.watch && a.text_dir.is_none() {
        return Err("--watch needs --text-dir".into());
    }
    let mut values = Vec::new();
    for f in &a.set {
        let (name, v) = f.split_once('=').ok_or_else(|| format!("--set {}: name=value", f))?;
        let index: i32 = name
            .strip_prefix("value")
            .and_then(|i| i.parse().ok())
            .ok_or_else(|| format!("--set {}: the only names are value0, value1, ... (the story values SETVALUE sets)", f))?;
        values.push((index, v.parse::<i32>().map_err(|_| format!("--set {}: not a number", f))?));
    }
    let mut s = Session::open_with(a.rom.clone(), a.save.clone(), a.overlay.clone(), a.data.clone().zip(a.lang.clone()))?;
    sunpetal_runtime::nds::set_wide(a.wide.unwrap_or(0));
    if let Some(n) = a.scale {
        sunpetal_runtime::nds::set_scale(n);
    }
    if let Some(p) = &a.pack_2d {
        let n = sunpetal_runtime::nds::set_pack_2d(Some(p))?;
        eprintln!("2D pack: {n} pictures");
    }
    for v in &a.pack_variant {
        let (g, n) = v.split_once('=').ok_or_else(|| format!("--pack-variant wants GROUP=VARIANT, got {v}"))?;
        sunpetal_runtime::nds::set_pack_2d_variant(g, n);
    }
    let mut extras = sunpetal_runtime::nds::Extras::default();
    if let Some(v) = a.outline {
        extras.outline = v;
    }
    if let Some(v) = a.outline_width {
        extras.outline_width = v;
    }
    if let Some(v) = a.shadow {
        extras.shadow = v;
    }
    if let Some(v) = a.shadow_radius {
        extras.shadow_radius = v;
    }
    if let Some(v) = a.msaa {
        extras.msaa = v;
    }
    if let Some(v) = &a.texture_filter {
        extras.filter = sunpetal_runtime::nds::TexFilter::parse(v);
    }
    if a.outline.is_some() || a.outline_width.is_some() || a.shadow.is_some() || a.shadow_radius.is_some() || a.msaa.is_some() || a.texture_filter.is_some() {
        sunpetal_runtime::nds::set_extras(extras);
    }
    match a.renderer.as_deref() {
        None | Some("software") => {}
        Some("gpu") => {
            if !cfg!(feature = "gpu") {
                eprintln!("--renderer gpu: this build has no `gpu` feature, the software engine draws");
            }
            sunpetal_runtime::nds::set_renderer(sunpetal_runtime::nds::Renderer::Gpu);
        }
        Some(v) => {
            eprintln!("unknown renderer {v} (software or gpu)");
            std::process::exit(2);
        }
    }
    let tracker = Tracker::install();
    if let Some(d) = &a.text_dir {
        s.use_texts(d, a.watch)?;
    }
    if let Some(p) = &a.dialogue_log {
        s.log_to(Some(p), a.append_log)?;
    }
    let mut scenes: Vec<String> = match (&a.scenes, &a.goto) {
        (Some(f), _) => std::fs::read_to_string(f)
            .map_err(|e| format!("{}: {}", f.display(), e))?
            .lines()
            .map(|l| l.split('#').next().unwrap_or("").trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        (None, Some(g)) => g.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        _ => Vec::new(),
    };
    if scenes.len() > 1 || (a.scenes.is_some() && !scenes.is_empty()) {
        return batch(&mut s, &a, &values, &scenes);
    }
    let scene = scenes.pop().or(a.chapter.map(|c| format!("C{:02}_01", c)));
    if let Some(scene) = &scene {
        s.goto(scene, values.iter().find(|v| v.0 == 0).map(|v| v.1))?;
        for (i, v) in values.iter().filter(|v| v.0 != 0) {
            s.set_value(*i, *v)?;
        }
        s.run(a.frame_delay)?;
    }
    let mut stop = None;
    if a.autoplay {
        stop = Some(s.autoplay(&Autoplay { max_lines: a.max_lines, delay: a.frame_delay, out: a.out.clone(), screens: a.screens_dir.clone() })?);
    }
    if let Some(p) = &a.shot {
        s.shot(p)?;
        if a.wide.is_some() {
            let wide = p.with_file_name(format!("{}_wide.png", p.file_stem().map_or(String::new(), |x| x.to_string_lossy().into_owned())));
            sunpetal_runtime::nds::screenshot_wide(&wide)?;
        }
    }
    if a.state || stop.is_some() {
        println!("{}", json!({"state": s.state(), "frame": s.vblank(), "map": s.map(), "value0": s.value(0), "stop": stop}));
    }
    if !a.repl {
        return if scene.is_some() || a.autoplay || a.shot.is_some() || a.state { Ok(()) } else { Err("nothing to do: give --goto, --autoplay, --shot, --state or --repl".into()) };
    }
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|e| format!("stdin: {}", e))?;
        if line.trim().is_empty() {
            continue;
        }
        let answer = match command(&mut s, &tracker, &line) {
            Ok(Some(v)) => v,
            Ok(None) => break,
            Err(e) => json!({"error": e}),
        };
        println!("{}", answer);
    }
    Ok(())
}

/// The scenes one after the other in one game run: the first from its request, a later one of the same chapter
/// from where the last left the game (a different chapter, or a game that does not come to a free walk, is a new
/// start of the chapter, by the request menu). A line of JSON on stdout for each: `status` is `ok` (the scene
/// ended or its lines were shown to the count asked), `stalled` (it waits for a walk, an event or a timer),
/// `no_event` (no event of the chapter starts it) or `not_started` (the hero stood at its place and the game did
/// not start it); the dialogue log, `--out` and the rest are common to all.
fn batch(s: &mut Session, a: &Args, values: &[(i32, i32)], scenes: &[String]) -> Result<(), String> {
    let value0 = values.iter().find(|v| v.0 == 0).map(|v| v.1);
    let mut last: Option<&str> = None;
    for (i, scene) in scenes.iter().enumerate() {
        let before = s.events().len();
        let chapter = scene.get(..3);
        let carry = last.is_some_and(|l| l.get(..3) == chapter && !scene.ends_with("_01"));
        let started = match carry {
            true => match s.goto_next(scene, value0) {
                Ok(true) => Ok(()),
                // the game did not settle: the rest of the list in a new run of the program
                Ok(false) => return rerun(a, &scenes[i..]),
                // the game is not as a run of the one scene would have it (a menu or a camera of the scene before)
                Err(e) if e.contains("did not start it") => return rerun(a, &scenes[i..]),
                Err(e) => Err(e),
            },
            false if last.is_some() => return rerun(a, &scenes[i..]),
            false => s.goto(scene, value0),
        };
        let started = started.and_then(|_| {
            for (i, v) in values.iter().filter(|v| v.0 != 0) {
                s.set_value(*i, *v)?;
            }
            s.run(a.frame_delay)
        });
        let (status, stop, error) = match started {
            Err(e) => (if e.contains("no event starts it") { "no_event" } else { "not_started" }, None, Some(e)),
            Ok(()) if a.autoplay => {
                let stop = s.autoplay(&Autoplay { max_lines: a.max_lines, delay: a.frame_delay, out: a.out.clone(), screens: a.screens_dir.clone() })?;
                (if stop == sunpetal_runtime::session::Stop::Stalled { "stalled" } else { "ok" }, Some(stop), None)
            }
            Ok(()) => ("ok", None, None),
        };
        let shown = s.events()[before..].iter().filter(|e| e.scene.as_deref() == Some(scene)).count();
        println!(
            "{}",
            json!({"scene": scene, "status": status, "stop": stop, "lines": shown, "state": s.state(), "frame": s.vblank(), "map": s.map(), "value0": s.value(0), "error": error})
        );
        last = Some(scene);
    }
    Ok(())
}

/// The scenes `rest` in a new run of this program (the game boots from its save again, as a run of one scene
/// does): the same arguments, the list in a file, the log added to. Its lines of JSON follow ours.
fn rerun(a: &Args, rest: &[String]) -> Result<(), String> {
    let list = std::env::temp_dir().join(format!("sunpetal-scenes-{}.txt", std::process::id()));
    std::fs::write(&list, rest.join("\n")).map_err(|e| format!("{}: {}", list.display(), e))?;
    let mut args: Vec<String> = Vec::new();
    let mut skip = false;
    for arg in std::env::args().skip(1) {
        if skip {
            skip = false;
        } else if arg == "--goto" || arg == "--scenes" {
            skip = true;
        } else if !arg.starts_with("--goto=") && !arg.starts_with("--scenes=") {
            args.push(arg);
        }
    }
    args.extend(["--scenes".into(), list.display().to_string()]);
    if a.dialogue_log.is_some() && !a.append_log {
        args.push("--append-log".into());
    }
    let status = std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?).args(&args).status().map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&list);
    if status.success() {
        Ok(())
    } else {
        Err(format!("the run of the rest of the list ended with {}", status))
    }
}

/// `sunpetal-debug frame REC N [--out DIR]` (K-099): everything the recording of a window session (`SUNPETAL_REC=1`, F10 or
/// the save key) holds of the DS frame N, read without running the game: the journal line, the 2D engines'
/// registers and OAM decoded, the frame's picture copied to `DIR/window.png`, `DIR/frame.json` written.
mod frame {
    use serde_json::{json, Value};
    use std::path::{Path, PathBuf};

    const KW: [[i64; 4]; 3] = [[8, 16, 32, 64], [16, 32, 32, 64], [8, 8, 16, 32]];
    const KH: [[i64; 4]; 3] = [[8, 16, 32, 64], [8, 8, 16, 32], [16, 32, 32, 64]];

    fn n(v: &Value) -> i64 {
        v.as_i64().unwrap_or(0)
    }

    /// One OAM entry `[index, a0, a1, a2]` as the engine reads it.
    pub fn sprite(e: &Value, dispcnt: i64) -> Value {
        let (i, a0, a1, a2) = (n(&e[0]), n(&e[1]), n(&e[2]), n(&e[3]));
        let rot = a0 & 0x100 != 0;
        let (shape, size) = ((a0 >> 14 & 3).min(2) as usize, (a1 >> 14 & 3) as usize);
        let (w, h) = (KW[shape][size], KH[shape][size]);
        let mut y = a0 & 255;
        let mut x = a1 & 511;
        if x >= 256 {
            x -= 512;
        }
        if y >= 192 {
            y -= 256;
        }
        let mode = ["normal", "blend", "window", "bitmap"][(a0 >> 10 & 3) as usize];
        let mut o = json!({
            "oam": i, "x": x, "y": y, "w": w, "h": h, "tile": a2 & 0x3FF, "palette": a2 >> 12 & 15,
            "priority": a2 >> 10 & 3, "mode": mode, "colors256": a0 & 0x2000 != 0, "mosaic": a0 & 0x1000 != 0,
            "affine": rot, "mapping_1d": dispcnt & 0x10 != 0,
        });
        if rot {
            o["double_size"] = json!(a0 & 0x200 != 0);
            o["matrix"] = json!(a1 >> 9 & 31);
        } else {
            o["flip_x"] = json!(a1 & 0x1000 != 0);
            o["flip_y"] = json!(a1 & 0x2000 != 0);
        }
        o
    }

    /// One engine's registers decoded.
    pub fn engine(v: &Value) -> Value {
        let d = n(&v["dispcnt"]);
        let layers: Vec<&str> = [(8, "bg0"), (9, "bg1"), (10, "bg2"), (11, "bg3"), (12, "obj"), (13, "win0"), (14, "win1"), (15, "objwin")]
            .iter()
            .filter(|(b, _)| d >> b & 1 != 0)
            .map(|(_, s)| *s)
            .collect();
        let bgcnt: Vec<Value> = v["bgcnt"]
            .as_array()
            .map(|a| {
                a.iter()
                    .enumerate()
                    .map(|(k, c)| {
                        let c = n(c);
                        json!({"bg": k, "priority": c & 3, "char_base": (c >> 2 & 15) * 0x4000, "mosaic": c & 0x40 != 0,
                               "colors256": c & 0x80 != 0, "screen_base": (c >> 8 & 31) * 0x800, "size_code": c >> 14 & 3,
                               "hofs": n(&v["hofs"][k]) & 0x1FF, "vofs": n(&v["vofs"][k]) & 0x1FF})
                    })
                    .collect()
            })
            .unwrap_or_default();
        let bld = n(&v["bldcnt"]);
        let targets = |m: i64| -> Vec<&str> { ["bg0", "bg1", "bg2", "bg3", "obj", "backdrop"].iter().enumerate().filter(|(i, _)| m >> i & 1 != 0).map(|(_, s)| *s).collect() };
        let bright = n(&v["bright"]);
        let bld_mode = ["off", "alpha", "brighten", "darken"][(bld >> 6 & 3) as usize];
        let bright_mode = ["off", "brighten", "darken", "off"][(bright >> 14 & 3) as usize];
        let sprites: Vec<Value> = v["spr"].as_array().map(|a| a.iter().map(|e| sprite(e, d)).collect()).unwrap_or_default();
        json!({
            "dispcnt": d, "bg_mode": d & 7, "display_mode": d >> 16 & 3, "forced_blank": d & 0x80 != 0, "layers_on": layers,
            "bgcnt": bgcnt,
            "blend": {"mode": bld_mode, "first": targets(bld & 63), "second": targets(bld >> 8 & 63),
                      "eva": n(&v["bldalpha"]) & 31, "evb": n(&v["bldalpha"]) >> 8 & 31, "evy": n(&v["bldy"]) & 31},
            "master_bright": {"mode": bright_mode, "factor": bright & 31},
            "sprites": sprites,
        })
    }

    /// The journal line of frame `vblank` in `dir/frames.jsonl`, or an error that says what the file holds.
    pub fn line(dir: &Path, vblank: u64) -> Result<Value, String> {
        let text = std::fs::read_to_string(dir.join("frames.jsonl")).map_err(|e| format!("{}: {}", dir.join("frames.jsonl").display(), e))?;
        let (mut first, mut last) = (u64::MAX, 0);
        for l in text.lines() {
            let Ok(v) = serde_json::from_str::<Value>(l) else { continue };
            let b = v["vblank"].as_u64().unwrap_or(0);
            first = first.min(b);
            last = last.max(b);
            if b == vblank {
                return Ok(v);
            }
        }
        Err(format!("frame {} is not in the recording (it holds {}..{})", vblank, first, last))
    }

    /// Everything about frame `vblank`; the picture goes to `out/window.png` and the answer to `out/frame.json`.
    pub fn run(rec: &Path, vblank: u64, out: Option<&Path>) -> Result<Value, String> {
        let j = line(rec, vblank)?;
        let mut a = json!({"vblank": vblank, "journal": j.clone()});
        let png = rec.join("frames").join(format!("v{:08}.png", vblank));
        a["window_png"] = json!(png.display().to_string());
        let have = j["ds_vblank"].as_u64();
        a["ds_state"] = match have {
            Some(b) if b == vblank => json!({"a": engine(&j["a"]), "b": engine(&j["b"])}),
            Some(b) => json!({"note": format!("the state is of frame {}: the window drew {} from it", b, vblank), "a": engine(&j["a"]), "b": engine(&j["b"])}),
            None => json!("not in this recording (made without registers: SUNPETAL_REC before K-099)"),
        };
        a["keys_hint"] = json!("the pictures' keys are in the F10 dump of the nearest frame (vN_2d/index.csv): match by OAM entry, x, y, w, h");
        if let Some(o) = out {
            std::fs::create_dir_all(o).map_err(|e| format!("{}: {}", o.display(), e))?;
            if png.exists() {
                std::fs::copy(&png, o.join("window.png")).map_err(|e| e.to_string())?;
            }
            std::fs::write(o.join("frame.json"), serde_json::to_string_pretty(&a).unwrap()).map_err(|e| e.to_string())?;
        }
        Ok(a)
    }

    /// `frame REC N [--out DIR]`
    pub fn cli(args: &[String]) -> Result<Value, String> {
        let usage = "usage: sunpetal-debug frame REC N [--out DIR]";
        let mut pos: Vec<&String> = Vec::new();
        let mut out: Option<PathBuf> = None;
        let mut it = args.iter();
        while let Some(x) = it.next() {
            if x == "--out" {
                out = Some(PathBuf::from(it.next().ok_or(usage)?));
            } else {
                pos.push(x);
            }
        }
        let [rec, n] = pos[..] else { return Err(usage.into()) };
        run(Path::new(rec), n.parse().map_err(|_| usage.to_string())?, out.as_deref())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn an_oam_entry_is_decoded() {
            // 16x32 at (184,80): shape 2 (tall), size 2; tile 5, palette 3, priority 1
            let a0 = 80 | 2 << 14;
            let a1 = 184 | 2 << 14;
            let a2 = 5 | 1 << 10 | 3 << 12;
            let s = sprite(&json!([7, a0, a1, a2]), 0x10);
            assert_eq!((s["x"].clone(), s["y"].clone(), s["w"].clone(), s["h"].clone()), (json!(184), json!(80), json!(16), json!(32)));
            assert_eq!((s["tile"].clone(), s["palette"].clone(), s["priority"].clone(), s["oam"].clone()), (json!(5), json!(3), json!(1), json!(7)));
            let neg = sprite(&json!([0, 250, 500, 0]), 0);
            assert_eq!((neg["x"].clone(), neg["y"].clone()), (json!(-12), json!(-6)));
        }

        #[test]
        fn registers_and_a_recording_are_read() {
            let e = engine(&json!({"dispcnt": 0x1_0000 | 0x1100 | 0x0100, "bgcnt": [1, 0, 0, 0], "hofs": [0, 0, 0, 0], "vofs": [0, 0, 0, 0],
                "bldcnt": 0x41, "bldalpha": 0x0408, "bldy": 0, "bright": 0x8004, "spr": []}));
            assert_eq!(e["layers_on"], json!(["bg0", "obj"]));
            assert_eq!(e["blend"]["mode"], json!("alpha"));
            assert_eq!((e["blend"]["eva"].clone(), e["blend"]["evb"].clone()), (json!(8), json!(4)));
            assert_eq!(e["master_bright"]["mode"], json!("darken"));
            let dir = std::env::temp_dir().join(format!("sunpetal-frame-{}", std::process::id()));
            std::fs::create_dir_all(dir.join("frames")).unwrap();
            std::fs::write(dir.join("frames").join("v00000011.png"), b"png").unwrap();
            std::fs::write(
                dir.join("frames.jsonl"),
                "{\"vblank\":10}\n{\"vblank\":11,\"ds_vblank\":11,\"a\":{\"dispcnt\":256,\"bgcnt\":[0,0,0,0],\"hofs\":[0,0,0,0],\"vofs\":[0,0,0,0],\"bldcnt\":0,\"bldalpha\":0,\"bldy\":0,\"bright\":0,\"spr\":[[0,0,0,0]]},\"b\":{\"dispcnt\":0}}\n",
            )
            .unwrap();
            let out = dir.join("out");
            let r = run(&dir, 11, Some(&out)).unwrap();
            assert_eq!(r["ds_state"]["a"]["sprites"].as_array().unwrap().len(), 1);
            assert!(out.join("window.png").exists() && out.join("frame.json").exists());
            assert_eq!(run(&dir, 10, None).unwrap()["ds_state"], json!("not in this recording (made without registers: SUNPETAL_REC before K-099)"));
            assert!(run(&dir, 99, None).unwrap_err().contains("10..11"));
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().map(String::as_str) == Some("frame") {
        return match frame::cli(&argv[1..]) {
            Ok(v) => {
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
                ExitCode::SUCCESS
            }
            Err(e) => {
                println!("{}", json!({"error": e}));
                ExitCode::from(1)
            }
        };
    }
    match run(Args::parse()) {
        // the threads' coroutines are parked: do not unwind them
        Ok(()) => std::process::exit(0),
        Err(e) => {
            // stdout is the machine's side: it gets the error too, as the repl does
            println!("{}", json!({"error": e}));
            eprintln!("{}", e);
            ExitCode::from(1)
        }
    }
}
