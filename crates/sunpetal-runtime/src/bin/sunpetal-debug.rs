//! Tools for proofreading the translation (docs/DEBUG-TOOLS-PLAN.md): runs the recompiled game
//! and reports on it as JSON lines on stdout and files. Needs `--features debug-tools`.
//!
//!     sunpetal-debug ROM.nds [--save FILE] [--repl]
//!
//! `--repl` reads commands from stdin, one per line, and answers each with a line of JSON
//! (`{"ok":true,...}` or `{"error":"..."}`):
//!
//!     run N | press KEYS [FRAMES] | tap X Y [FRAMES] | shot FILE
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
    /// Commands from stdin, one per line.
    #[arg(long)]
    repl: bool,
    /// Write one JSON line per dialogue window opened to this file.
    #[arg(long, value_name = "FILE")]
    dialogue_log: Option<PathBuf>,
    /// Play to this scene (Cnn_yy) through the menu of requests first.
    #[arg(long, value_name = "Cnn_yy")]
    goto: Option<String>,
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
            s.log_to(w.first().filter(|p| **p != "off").map(std::path::Path::new))?;
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
    let mut s = Session::open(a.rom, a.save)?;
    let tracker = Tracker::install();
    if let Some(d) = &a.text_dir {
        s.use_texts(d, a.watch)?;
    }
    if let Some(p) = &a.dialogue_log {
        s.log_to(Some(p))?;
    }
    let scene = a.goto.clone().or(a.chapter.map(|c| format!("C{:02}_01", c)));
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
