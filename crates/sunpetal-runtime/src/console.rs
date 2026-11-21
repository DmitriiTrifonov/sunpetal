//! The commands of the window's console (K-232), run on the machine's thread between frames: the same names and
//! arguments as `sunpetal-debug`'s REPL (`setvalue`, `value`, `teleport`, `map`, `scene`, `cam`, `where`). `goto`
//! and `shot` need the app (a restart through the menu of requests; the window's picture) and are not here.
use crate::{nativecam, nds, session};

/// The commands, for `help` and the console's completion.
pub const COMMANDS: &[(&str, &str)] = &[
    ("goto", "goto Cnn_yy: Cnn_01 starts the chapter in place; a later scene restarts and plays to it"),
    ("chapter", "chapter N: start chapter N from here (no menu, no restart)"),
    ("map", "map N [X Y Z]: the hero to map N (X Y Z in 1/4096; the same place if none)"),
    ("teleport", "teleport X Y Z: the hero to the place (1/4096)"),
    ("setvalue", "setvalue I V: story value I = V (0 is the chapter's step)"),
    ("value", "value [I]: story value I (default 0)"),
    ("where", "where: scene, map, place, story step"),
    ("charwalk", "charwalk SCENE [AREA]: the d-pad walks the hero by the PS2 scene's walls and floor; `charwalk off` ends it"),
    ("ps2room", "ps2room SCENE ROOM | auto: the PS2 camera and walk of a room of rooms.tsv together (auto: the room of each DS map by maps.tsv); `ps2room off` ends both"),
    ("fastrun", "fastrun on|off: the fast run of the PS2 walk allowed (an event does it on PS2; zones near event points still forbid it)"),
    ("zone", "zone hit|height N on|off: a wall or floor zone of the PS2 walk switched (the game's events do it on PS2)"),
    ("scene", "scene: the script file the game read last"),
    ("cam", "cam YAW PITCH [DX DY DZ]: the scene camera turned (degrees) and moved; `cam 0 0` is the game's"),
    ("shot", "shot: the window's picture and its parts to artifacts/shots"),
    ("bookmark", "bookmark NAME: the place kept (input from the start, the start save, the checkpoints) in artifacts/bookmarks/NAME"),
    ("help", "help: this list"),
];

fn num<T: std::str::FromStr>(w: &[&str], i: usize, name: &str, default: Option<T>) -> Result<T, String> {
    match w.get(i) {
        Some(s) => s.parse().map_err(|_| format!("{}: not a number: {}", name, s)),
        None => default.ok_or_else(|| format!("{} is missing", name)),
    }
}

/// The game's mode controller (`0x0201C544`, once a frame): `[G]` the task of the mode that runs, `[G+4]` the mode asked for
/// (0: none), `[G+8]` its parameter. The mode table (`0x02061E14`, 8 bytes a row: overlay, callbacks) has the world, the
/// game proper, at row 2.
const MODE_CONTROL: u32 = 0x0206_9B64;
const MODE_WORLD: u32 = 2;
/// The chapter the world starts (`0x0204E72C` stores it and a flag; the world's entry reads both).
const CHAPTER: u32 = 0x0207_8638;
const CHAPTER_FLAG: u32 = 0x0207_8658;

fn word(a: u32) -> Option<u32> {
    nds::debug::peek(a, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Starts chapter `n` from wherever the game is, as the menu of requests does (K-234, docs/GOTO-NATIVE.md): the chapter
/// is put where the world reads it, the running mode is told to end (the two words its task is kept going by are
/// cleared, and the controller takes it for done on the next frame), and the world is asked for as the next mode.
/// Call between frames. The scene `Cn_01` is read some 20 frames later.
pub fn start_chapter(n: u32) -> Result<(), String> {
    if !(1..=19).contains(&n) {
        return Err(format!("chapter {}: 1 to 19", n));
    }
    let task = word(MODE_CONTROL).filter(|t| *t != 0).ok_or("no mode is running (the game has not booted)")?;
    let ok = nds::debug::poke(CHAPTER, &n.to_le_bytes())
        && nds::debug::poke(CHAPTER_FLAG, &1u32.to_le_bytes())
        && nds::debug::poke(task + 0x14, &[0; 8])
        && nds::debug::poke(MODE_CONTROL + 4, &MODE_WORLD.to_le_bytes())
        && nds::debug::poke(MODE_CONTROL + 8, &0u32.to_le_bytes());
    ok.then_some(()).ok_or_else(|| "the controller's memory is not there".into())
}

/// `bookmark NAME` (K-221, А1.6): the session's recording of input (`SUNPETAL_INPUT_REC`, which the window starts with the debug
/// menu), the save it started from and its checkpoints, copied to `artifacts/bookmarks/NAME`, with where the hero is; and a line
/// in `artifacts/bookmarks/places.toml` for `tools/auto/places.py`. A replay with `sunpetal-headless` comes back to this frame.
fn bookmark(name: &str) -> Result<String, String> {
    use std::path::Path;
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err("NAME: letters, digits, _ and - only".into());
    }
    let rec = std::env::var_os("SUNPETAL_INPUT_REC").ok_or("no recording runs: start the window with SUNPETAL_DEBUG_MENU=1")?;
    let rec = Path::new(&rec);
    let root = rec.parent().and_then(Path::parent).ok_or("the recording has no folder")?;
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let copy = |from: &Path, to: &str| std::fs::copy(from, dir.join(to)).map(|_| ()).map_err(|e| format!("{}: {}", from.display(), e));
    nds::flush_save();
    copy(rec, "input.txt")?;
    copy(&rec.with_file_name("start.sav"), "start.sav")?;
    copy(&rec.with_extension("chk"), "input.chk")?;
    let frame = nds::vblanks();
    let (scene, map) = (nds::script_scene(), session::hero_map());
    let meta = serde_json::json!({
        "name": name,
        "frame": frame,
        "scene": scene,
        "map": map,
        "pos": session::hero_pos(),
        "angle": session::hero_angle(),
        "step": session::story_value(0),
    })
    .to_string()
        + "\n";
    std::fs::write(dir.join("meta.json"), meta).map_err(|e| e.to_string())?;
    let mut line = format!("\n[[place]]\nid = {:?}\ndesc = \"закладка автора\"\nreplay = {:?}\nframes = {}\n", name, dir.display().to_string(), frame);
    let expect: Vec<String> = scene.iter().map(|s| format!("scene = {:?}", s)).chain(map.iter().map(|m| format!("map = {}", m))).collect();
    line.push_str(&format!("expect = {{ {} }}\n", expect.join(", ")));
    use std::io::Write;
    std::fs::OpenOptions::new().create(true).append(true).open(root.join("places.toml")).and_then(|mut f| f.write_all(line.as_bytes())).map_err(|e| e.to_string())?;
    Ok(format!("bookmark {} at frame {} in {}", name, frame, dir.display()))
}

/// Runs one machine-side command; the answer, one line per `\n`. Call between frames.
pub fn run(line: &str) -> Result<String, String> {
    let w: Vec<&str> = line.split_whitespace().collect();
    let Some((&name, w)) = w.split_first() else { return Ok(String::new()) };
    match name {
        "help" | "?" => Ok(COMMANDS.iter().map(|(_, h)| *h).collect::<Vec<_>>().join("\n")),
        "value" => {
            let i = num(w, 0, "INDEX", Some(0u32))?;
            session::story_value(i).map(|v| format!("value {} = {}", i, v)).ok_or_else(|| "no memory there".into())
        }
        "setvalue" => {
            session::queue_set_value(num(w, 0, "INDEX", None)?, num(w, 1, "VALUE", None)?);
            Ok("queued".into())
        }
        "teleport" => {
            let p = [num(w, 0, "X", None)?, num(w, 1, "Y", None)?, num(w, 2, "Z", None)?];
            if session::hero_teleport(p) {
                Ok(format!("hero at {:?}", p))
            } else {
                Err("no hero on this map yet".into())
            }
        }
        "map" => {
            let map: i32 = num(w, 0, "MAP", None)?;
            let pos = match w.len() {
                1 => session::hero_pos().ok_or("no hero: give X Y Z")?,
                _ => [num(w, 1, "X", None)?, num(w, 2, "Y", None)?, num(w, 3, "Z", None)?],
            };
            session::queue_change_map(map, pos);
            Ok(format!("map {} at {:?} queued", map, pos))
        }
        "chapter" => {
            start_chapter(num(w, 0, "N", None)?)?;
            Ok("chapter asked for; the scene starts in a few frames".into())
        }
        "ps2room" => {
            if w.first() == Some(&"off") {
                crate::nativewalk::auto_off();
                return Ok("the game's own camera and walk".into());
            }
            let root = std::env::var("SUNPETAL_EE_EXPORT").map_err(|_| "SUNPETAL_EE_EXPORT is not set")?;
            if w.first() == Some(&"auto") {
                crate::nativewalk::auto_on(&root)?;
                return Ok("PS2 room follows the DS map".into());
            }
            crate::nativewalk::room(&root, w.first().ok_or("SCENE is missing")?, num(w, 1, "ROOM", None)?)?;
            Ok("PS2 room on".into())
        }
        "fastrun" => {
            crate::nativewalk::set_fast_mode(match w.first() {
                Some(&"on") => true,
                Some(&"off") => false,
                _ => return Err("fastrun on|off".into()),
            })?;
            Ok("fast run set".into())
        }
        "zone" => {
            let on = match w.get(2) {
                Some(&"on") => true,
                Some(&"off") => false,
                _ => return Err("zone hit|height N on|off".into()),
            };
            crate::nativewalk::set_zone(w.first().ok_or("KIND is missing")?, num(w, 1, "N", None)?, on)?;
            Ok("zone set".into())
        }
        "charwalk" => {
            // charwalk SCENE [AREA]: the d-pad walks the hero on the PS2 scene's walls and floor (SUNPETAL_EE_EXPORT); `charwalk off`
            if w.first() == Some(&"off") {
                crate::nativewalk::disable();
                return Ok("the game's own walk".into());
            }
            let root = std::env::var("SUNPETAL_EE_EXPORT").map_err(|_| "SUNPETAL_EE_EXPORT is not set")?;
            crate::nativewalk::enable(&root, w.first().ok_or("SCENE is missing")?, num(w, 1, "AREA", Some(0usize))?)?;
            Ok("PS2 walk on".into())
        }
        "scene" => Ok(nds::script_scene().unwrap_or_else(|| "(none yet)".into())),
        "bookmark" => bookmark(w.first().ok_or("NAME is missing")?),
        "where" => Ok(format!(
            "scene {}  map {:?}  pos {:?}  angle {:?}  step {:?}  frame {}",
            nds::script_scene().unwrap_or_else(|| "-".into()),
            session::hero_map(),
            session::hero_pos(),
            session::hero_angle(),
            session::story_value(0),
            nds::vblanks()
        )),
        "cam" if w.first() == Some(&"ps2") => {
            // cam ps2 EX EY EZ PITCH YAW ROLL [T]: the PS2 camera (place in PS2 units, radians); `cam ps2 off` ends it
            if w.get(1) == Some(&"off") {
                nativecam::clear_ps2();
                return Ok("the game's own camera".into());
            }
            let n = |i: usize, name: &str| num(&w[1..], i, name, None);
            nativecam::set_ps2([n(0, "EX")?, n(1, "EY")?, n(2, "EZ")?], n(3, "PITCH")?, n(4, "YAW")?, n(5, "ROLL")?, [16.0, -18.5996, -13.0], w.get(7) == Some(&"t"));
            Ok("PS2 camera set".into())
        }
        "cam" => {
            nativecam::set(num(w, 0, "YAW", Some(0.0))?, num(w, 1, "PITCH", Some(0.0))?, [num(w, 2, "DX", Some(0.0))?, num(w, 3, "DY", Some(0.0))?, num(w, 4, "DZ", Some(0.0))?]);
            Ok(if w.len() >= 2 && w[0] == "0" && w[1] == "0" { "the game's own camera".into() } else { "camera set".into() })
        }
        _ => Err(format!("{}: no such command (help)", name)),
    }
}
