//! The PS2 walk on the DS hero, first hook (K-236): the d-pad no longer reaches the game; each frame the hero's place
//! (in the game's memory) goes by `sunpetal_data::ps2::walk` on the PS2 scene's walls and floor, relative to the camera.
//! A prototype: the hero slides without the walk animation, the face turns by the game's rule. `walk ps2 SCENE AREA` / `walk off`.
use crate::{nativecam, session};
use std::sync::Mutex;
use sunpetal_data::ps2::export::{Area3d, Export};
use sunpetal_data::ps2::walk::{walk_frame, Speeds, Walker, SELEKA};

/// The PS2 -> DS shift of the world; the export's units are tenths of it.
const SHIFT: [f64; 3] = [16.0, -18.5996, -13.0];
/// The d-pad's bits in the keys word (`KEY_NAMES`: A B SELECT START RIGHT LEFT UP DOWN ...).
const RIGHT: u32 = 1 << 4;
const LEFT: u32 = 1 << 5;
const UP: u32 = 1 << 6;
const DOWN: u32 = 1 << 7;

struct Walk {
    area: Area3d,
    /// where the hero was put last (export units), to start the next step from
    last: Option<[f32; 3]>,
    /// the DS place written last, to tell a move of the game's (a new map) from ours
    written: Option<[i32; 3]>,
    /// the face (0 is +z, π/2 is +x) and the turn it is in
    walker: Walker,
}
static WALK: Mutex<Option<Walk>> = Mutex::new(None);

/// Switches the walk on for the area `area` (index in `scenes/<scene>.json`'s `scene3d`) of the export at `root`.
pub fn enable(root: &str, scene: &str, area: usize) -> Result<(), String> {
    let data = Export::open(root).and_then(|e| e.scene(scene)).map_err(|e| e.to_string())?;
    let index = area;
    let mut area = data.areas.into_iter().nth(area).ok_or("no such area")?;
    // the walls and floors a new game has on (zones.tsv); the events that switch them later are not followed
    let zones = std::fs::read_to_string(std::path::Path::new(root).join("zones.tsv")).map_err(|e| format!("zones.tsv: {}", e))?;
    sunpetal_data::ps2::walk::apply_initial_zones(&mut area, &zones, scene, index);
    *WALK.lock().unwrap() = Some(Walk { area, last: None, written: None, walker: Walker::default() });
    Ok(())
}

/// A room of the export (`rooms.tsv`: scene and index) on the DS hero: the PS2 camera after the hero and the PS2 walk, together.
/// The room's index is its camera record and its Scene3D area both (Y-25). Fails if the table has no such room.
pub fn room(root: &str, scene: &str, index: usize) -> Result<(), String> {
    let table = std::fs::read_to_string(std::path::Path::new(root).join("rooms.tsv")).map_err(|e| format!("rooms.tsv: {}", e))?;
    let known = table.lines().skip(1).any(|l| {
        let mut c = l.split('\t');
        c.next() == Some(scene) && c.next().and_then(|i| i.parse::<usize>().ok()) == Some(index)
    });
    if !known {
        return Err(format!("rooms.tsv has no room {} {}", scene, index));
    }
    nativecam::follow_ps2_path(root, scene, index, None)?;
    enable(root, scene, index)
}

/// Switches one wall (`hit`) or floor (`height`) zone of the walking room on or off, as the PS2 events `SetHitEnabled`/`SetHeightEnabled` do
/// (`hit.md` §7: the byte's bit 0). The game's own events are PS2 scripts that this run does not execute, so zones follow by hand.
pub fn set_zone(kind: &str, zone: usize, on: bool) -> Result<(), String> {
    let mut g = WALK.lock().unwrap();
    let w = g.as_mut().ok_or("the PS2 walk is off")?;
    match kind {
        "hit" => w.area.hit.get_mut(zone).map(|h| h.enabled = on),
        "height" => w.area.height.get_mut(zone).map(|t| t.enabled = on),
        _ => return Err("kind is hit or height".into()),
    }
    .ok_or_else(|| format!("the room has no {} zone {}", kind, zone))
}

/// Back to the game's own camera and walk.
pub fn room_off() {
    nativecam::clear_ps2();
    disable();
}

pub fn disable() {
    *WALK.lock().unwrap() = None;
}

pub fn active() -> bool {
    WALK.lock().unwrap().is_some()
}

/// The keys the game gets: the d-pad is ours while the walk is on. Moves the hero for this frame's d-pad.
pub fn frame(keys: u32) -> u32 {
    let mut g = WALK.lock().unwrap();
    let Some(w) = g.as_mut() else { return keys };
    if let Some(pos) = session::hero_pos() {
        let ds = |i: usize| pos[i] as f64 / 4096.0;
        let here = [((ds(0) - SHIFT[0]) * 10.0) as f32, ((ds(1) - SHIFT[1]) * 10.0) as f32, ((ds(2) - SHIFT[2]) * 10.0) as f32];
        // the hero is where we put it, or somewhere else for the game's reasons (a scene, a teleport): start from there
        let ours = w.written.is_some_and(|k| (0..3).all(|i| (k[i] - pos[i]).abs() < 8));
        let from = if ours { w.last.unwrap_or(here) } else { here };
        let (dx, dy) = ((keys & RIGHT != 0) as i8 - (keys & LEFT != 0) as i8, (keys & DOWN != 0) as i8 - (keys & UP != 0) as i8);
        let stick = (dx * 127, dy * 127);
        if !ours {
            w.walker = Walker { face: session::hero_angle().map_or(0.0, |a| (a & 0xFFFF) as f32 / 65536.0 * std::f32::consts::TAU), ..Walker::default() };
        }
        // the game's state machine: the first frame of a walk and of a big turn stand, a turn holds the walk speed
        let used = w.walker.frame(stick);
        let to = match used {
            0 => from,
            1 => walk_frame(&w.area, from, stick, nativecam::yaw(), Speeds { walk: 0.25, run: 0.25 }),
            _ => walk_frame(&w.area, from, stick, nativecam::yaw(), Speeds { walk: SELEKA.walk, run: SELEKA.run }),
        };
        let out = [(to[0] as f64 / 10.0 + SHIFT[0]) * 4096.0, (to[1] as f64 / 10.0 + SHIFT[1]) * 4096.0, (to[2] as f64 / 10.0 + SHIFT[2]) * 4096.0];
        let out = [out[0].round() as i32, out[1].round() as i32, out[2].round() as i32];
        if stick != (0, 0) {
            session::hero_set_angle((w.walker.face.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU * 65536.0) as u32 & 0xFFFF);
        }
        if stick != (0, 0) && session::hero_teleport(out) {
            w.last = Some(to);
            w.written = Some(out);
        } else if stick == (0, 0) {
            w.last = Some(from);
            w.written = Some(pos);
        }
    }
    keys & !(RIGHT | LEFT | UP | DOWN)
}
