//! Tools for proofreading the translation (docs/DEBUG-TOOLS-PLAN.md): runs the recompiled game
//! and reports on it as JSON lines on stdout and files. Needs `--features debug-tools`.
//!
//!     sunpetal-debug ROM.nds [--save FILE] [--repl]
//!
//! `--repl` reads commands from stdin, one per line, and answers each with a line of JSON
//! (`{"ok":true,...}` or `{"error":"..."}`):
//!
//!     run N | press KEYS [FRAMES] | tap X Y [FRAMES] | shot FILE | shotwide FILE (with --wide)
//!     log FILE|off | advance [FRAMES] | walk N [FRAMES] | shown | peek ADDR[:LEN] | find TEXT | findhex HEX | calls FRAMES FILE [KEYS] | callsarg FRAMES LO[-HI] [KEYS] | quit
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
    /// Both screens as a PNG at the end.
    #[arg(long, value_name = "FILE")]
    shot: Option<PathBuf>,
    /// Widen the 3D view by this many DS pixels each side; `--shot` then also writes FILE with `_wide` before the
    /// extension (the upper screen alone, 256 + 2N wide).
    #[arg(long, value_name = "PIXELS")]
    wide: Option<usize>,
    /// The 3D's internal resolution (1-8 times the DS's) for `--shot`.
    #[arg(long, value_name = "N")]
    scale: Option<usize>,
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
        "scriptstate" => done(json!({"loaded": sunpetal_runtime::nds::script_loaded(), "idle": sunpetal_runtime::nds::script_idle(), "waits": sunpetal_runtime::nds::script_waits(), "mark": sunpetal_runtime::nds::script_mark()})),
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
        "shot" => {
            s.shot(std::path::Path::new(w.first().ok_or("shot FILE")?))?;
            done(json!({}))
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
            let bytes: Vec<u8> = (0..rest.len() / 2).map(|i| u8::from_str_radix(&rest[2 * i..2 * i + 2], 16).map_err(|_| format!("{}: not hex", rest))).collect::<Result<_, _>>()?;
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
        let index: i32 = name.strip_prefix("value").and_then(|i| i.parse().ok()).ok_or_else(|| format!("--set {}: the only names are value0, value1, ... (the story values SETVALUE sets)", f))?;
        values.push((index, v.parse::<i32>().map_err(|_| format!("--set {}: not a number", f))?));
    }
    let mut s = Session::open_with(a.rom.clone(), a.save.clone(), a.overlay.clone(), a.data.clone().zip(a.lang.clone()))?;
    sunpetal_runtime::nds::set_wide(a.wide.unwrap_or(0));
    if let Some(n) = a.scale {
        sunpetal_runtime::nds::set_scale(n);
    }
    let tracker = Tracker::install();
    if let Some(d) = &a.text_dir {
        s.use_texts(d, a.watch)?;
    }
    if let Some(p) = &a.dialogue_log {
        s.log_to(Some(p), a.append_log)?;
    }
    let mut scenes: Vec<String> = match (&a.scenes, &a.goto) {
        (Some(f), _) => std::fs::read_to_string(f).map_err(|e| format!("{}: {}", f.display(), e))?.lines().map(|l| l.split('#').next().unwrap_or("").trim().to_string()).filter(|l| !l.is_empty()).collect(),
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
        stop = Some(s.autoplay(&Autoplay { max_lines: a.max_lines, delay: a.frame_delay, out: a.out.clone() })?);
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
                let stop = s.autoplay(&Autoplay { max_lines: a.max_lines, delay: a.frame_delay, out: a.out.clone() })?;
                (if stop == sunpetal_runtime::session::Stop::Stalled { "stalled" } else { "ok" }, Some(stop), None)
            }
            Ok(()) => ("ok", None, None),
        };
        let shown = s.events()[before..].iter().filter(|e| e.scene.as_deref() == Some(scene)).count();
        println!("{}", json!({"scene": scene, "status": status, "stop": stop, "lines": shown, "state": s.state(), "frame": s.vblank(), "map": s.map(), "value0": s.value(0), "error": error}));
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
    if status.success() { Ok(()) } else { Err(format!("the run of the rest of the list ended with {}", status)) }
}

fn main() -> ExitCode {
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
