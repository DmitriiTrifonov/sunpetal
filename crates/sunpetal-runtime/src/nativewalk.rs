//! The PS2 walk on the DS hero, first hook (K-236): the d-pad no longer reaches the game; each frame the hero's place
//! (in the game's memory) goes by `sunpetal_data::ps2::walk` on the PS2 scene's walls and floor, relative to the camera.
//! A prototype: the hero slides without the walk animation, the face turns by the game's rule. `walk ps2 SCENE AREA` / `walk off`.
use crate::{nativecam, session};
use std::sync::Mutex;
use sunpetal_data::ps2::export::{Area3d, Export};
use sunpetal_data::ps2::walk::{fast_run_allowed, walk_frame, Speeds, Walker, SELEKA, SELEKA_FAST};

/// The d-pad's bits in the keys word (`KEY_NAMES`: A B SELECT START RIGHT LEFT UP DOWN ...).
const RIGHT: u32 = 1 << 4;
const LEFT: u32 = 1 << 5;
const UP: u32 = 1 << 6;
const DOWN: u32 = 1 << 7;

struct Walk {
    area: Area3d,
    /// the room: its PS2 scene and Scene3D index
    room: (String, usize),
    /// where the hero was put last (export units), to start the next step from
    last: Option<[f32; 3]>,
    /// the DS place written last, to tell a move of the game's (a new map) from ours
    written: Option<[i32; 3]>,
    /// the face (0 is +z, π/2 is +x) and the turn it is in
    walker: Walker,
    /// the fast run is let by an event and the run mode is on (`WChara+0xcd`, `+0x13` bit 1; the game's events set them, here `fastrun on`)
    fast_mode: bool,
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
    *WALK.lock().unwrap() = Some(Walk { area, room: (scene.to_string(), index), fast_mode: false, last: None, written: None, walker: Walker::default() });
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
    // the hero's map gives the shift of the PS2 world (each map is aligned on its own: A46 by 16, most by none)
    let map = session::hero_map().ok_or("the hero's map is not known")?;
    nativecam::set_shift(sunpetal_data::ps2::export::area_shift(map as u32).ok_or_else(|| format!("map {} has no PS2 alignment in areas.toml", map))?);
    nativecam::follow_ps2_path(root, scene, index, None)?;
    enable(root, scene, index)
}

/// `ps2room auto`: on each map the DS game enters, the room of that map from `maps.tsv` (Y-38) is put on, by the DS scene's chapter;
/// a map with no room leaves the game's own camera and walk. Call once a frame.
type Room = Option<(String, usize)>;
struct Auto {
    root: String,
    rows: Vec<sunpetal_data::ps2::export::MapRow>,
    /// the map and scene the room was last chosen for, and the choice
    last: Option<(u32, Option<String>, Room)>,
}
static AUTO: Mutex<Option<Auto>> = Mutex::new(None);

pub fn auto_on(root: &str) -> Result<(), String> {
    let rows = Export::open(root).and_then(|e| e.maps())?;
    *AUTO.lock().unwrap() = Some(Auto { root: root.to_string(), rows, last: None });
    Ok(())
}

pub fn auto_off() {
    *AUTO.lock().unwrap() = None;
    room_off();
}

pub fn auto_tick() {
    // a new DS scene read: the zones its event switches (the room's own, whether chosen by hand or by `auto`)
    static SEEN: Mutex<Option<String>> = Mutex::new(None);
    let now = crate::nds::script_scene();
    {
        let mut seen = SEEN.lock().unwrap();
        if *seen != now {
            *seen = now.clone();
            if let Some(s) = &now {
                scene_started(s);
            }
        }
    }
    let mut g = AUTO.lock().unwrap();
    let Some(auto) = g.as_mut() else { return };
    let Some(map) = session::hero_map() else { return };
    // the PS2 camera and walk are for free walking: a running script (a cutscene) has the game's own camera
    if crate::nds::script_idle() != Some(true) {
        if auto.last.take().is_some() {
            drop(g);
            room_off();
        }
        return;
    }
    let scene = crate::nds::script_scene();
    if auto.last.as_ref().is_some_and(|(m, s, _)| *m == map as u32 && *s == scene) {
        return;
    }
    let want = auto.rows.iter().find(|r| r.map == map as u32).and_then(|r| r.room_for(scene.as_deref()));
    // another scene of the same map that chooses the same room leaves the walk as it is
    let same = auto.last.as_ref().is_some_and(|(m, _, w)| *m == map as u32 && *w == want);
    auto.last = Some((map as u32, scene, want.clone()));
    let root = auto.root.clone();
    drop(g);
    if same {
        return;
    }
    match want {
        Some((s, i)) => match room(&root, &s, i) {
            Ok(()) => eprintln!("ps2room auto: map {} -> {} room {}", map, s, i),
            Err(e) => {
                eprintln!("ps2room auto: map {}: {}", map, e);
                room_off();
            }
        },
        None => {
            room_off();
            eprintln!("ps2room auto: map {} has no PS2 room", map);
        }
    }
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

/// The zone switches a DS scene makes (`data/ult/zone_switch.tsv`, from `tools/ult/event_match.py`): (PS2 scene, room, kind, zone, on).
fn switches(ds_scene: &str) -> Vec<(&'static str, usize, &'static str, usize, bool)> {
    static T: &str = include_str!("../../../data/ult/zone_switch.tsv");
    T.lines()
        .skip(1)
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            (c.len() == 7 && c[0] == ds_scene).then(|| Some((c[1], c[2].parse().ok()?, c[3], c[4].parse().ok()?, c[5] == "on")))?
        })
        .collect()
}

/// A DS scene has started: the walls and floors of the walking room that the PS2 event of that scene (found by its place) switches follow it.
pub fn scene_started(ds_scene: &str) -> usize {
    let mut g = WALK.lock().unwrap();
    let Some(w) = g.as_mut() else { return 0 };
    let mut n = 0;
    for (scene, index, kind, zone, on) in switches(ds_scene) {
        if w.room.0 == scene && w.room.1 == index {
            match kind {
                "hit" => w.area.hit.get_mut(zone).map(|h| h.enabled = on),
                _ => w.area.height.get_mut(zone).map(|t| t.enabled = on),
            };
            n += 1;
            eprintln!("ps2room: {} switches {} {} {} of {} {}", ds_scene, kind, zone, if on { "on" } else { "off" }, scene, index);
        }
    }
    n
}

/// The walking room and which of its walls and floors are on (for the debugger): (scene, index, hit flags, height flags).
pub fn zone_state() -> Option<(String, usize, Vec<bool>, Vec<bool>)> {
    let g = WALK.lock().unwrap();
    let w = g.as_ref()?;
    Some((w.room.0.clone(), w.room.1, w.area.hit.iter().map(|h| h.enabled).collect(), w.area.height.iter().map(|t| t.enabled).collect()))
}

/// The fast run allowed or not (`walk.md` §3a); its zones that forbid it stay as the room has them.
pub fn set_fast_mode(on: bool) -> Result<(), String> {
    WALK.lock().unwrap().as_mut().ok_or("the PS2 walk is off")?.fast_mode = on;
    Ok(())
}

/// The hero's move state the last frame left (0 stands, 1 walks, 2 runs, 3 runs fast), while the PS2 walk is on: the camera's dead zone follows it.
pub fn move_state() -> Option<u8> {
    WALK.lock().unwrap().as_ref().map(|w| w.walker.state)
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
        let sh = nativecam::shift();
        let here = [((ds(0) - sh[0]) * 10.0) as f32, ((ds(1) - sh[1]) * 10.0) as f32, ((ds(2) - sh[2]) * 10.0) as f32];
        // the hero is where we put it, or somewhere else for the game's reasons (a scene, a teleport): start from there
        let ours = w.written.is_some_and(|k| (0..3).all(|i| (k[i] - pos[i]).abs() < 8));
        let from = if ours { w.last.unwrap_or(here) } else { here };
        let (dx, dy) = ((keys & RIGHT != 0) as i8 - (keys & LEFT != 0) as i8, (keys & DOWN != 0) as i8 - (keys & UP != 0) as i8);
        let stick = (dx * 127, dy * 127);
        if !ours {
            w.walker = Walker { face: session::hero_angle().map_or(0.0, |a| (a & 0xFFFF) as f32 / 65536.0 * std::f32::consts::TAU), ..Walker::default() };
        }
        // the game's state machine: the first frame of a walk and of a big turn stand, a turn holds the walk speed
        let fast = w.fast_mode && fast_run_allowed(&w.area, from);
        let used = w.walker.frame_fast(stick, fast);
        let to = match used {
            0 => from,
            1 => walk_frame(&w.area, from, stick, nativecam::yaw(), Speeds { walk: 0.25, run: 0.25 }),
            3 => walk_frame(&w.area, from, stick, nativecam::yaw(), Speeds { walk: SELEKA.walk, run: SELEKA_FAST }),
            _ => walk_frame(&w.area, from, stick, nativecam::yaw(), Speeds { walk: SELEKA.walk, run: SELEKA.run }),
        };
        let out = [(to[0] as f64 / 10.0 + sh[0]) * 4096.0, (to[1] as f64 / 10.0 + sh[1]) * 4096.0, (to[2] as f64 / 10.0 + sh[2]) * 4096.0];
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_gives_its_zone_switches_and_a_stranger_gives_none() {
        let s = switches("C02_04");
        assert!(s.contains(&("R02", 0, "height", 2, false)) && s.contains(&("R02", 0, "height", 0, true)));
        assert!(switches("no such scene").is_empty());
    }
}
