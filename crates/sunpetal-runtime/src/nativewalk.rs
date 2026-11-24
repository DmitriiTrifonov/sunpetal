//! The PS2 walk on the DS hero, first hook (K-236): the d-pad no longer reaches the game; each frame the hero's place
//! (in the game's memory) goes by `sunpetal_data::ps2::walk` on the PS2 scene's walls and floor, relative to the camera.
//! A prototype: the hero slides without the walk animation, the face turns by the game's rule. `walk ps2 SCENE AREA` / `walk off`.
use crate::{nativecam, session};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Mutex;
use sunpetal_data::ps2::export::{Area3d, Export};
use sunpetal_data::ps2::walk::{auto_events, fast_run_allowed, walk_frame, Speeds, Walker, SELEKA, SELEKA_FAST};

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
    /// the automatic event zones the hero stood in last frame (an event fires on walking in)
    inside: Vec<usize>,
    /// the stairs' floor the zones were last set for (map A30)
    floor: Option<u32>,
}
static WALK: Mutex<Option<Walk>> = Mutex::new(None);

/// The fast run let by an event (`SetFastRunModeOperationChara`, `WChara+0xcd`) and the hero's run mode (`ChangeRunModeChara` of
/// chara 0, `+0x13` bit 1): the fast run wants both. The game keeps them in its chara manager across rooms; so here.
static FAST_LET: AtomicBool = AtomicBool::new(false);
static RUN_MODE: AtomicBool = AtomicBool::new(true);

/// A run mode switch of `data/ult/run_mode.tsv` (`sunpetal-ee` Y-58): whether it is the fast run's (else the run mode's) and the value.
type RunSet = (bool, bool);

/// The run mode switches of the PS2 events, in the table's order (event, pc): `room` the init event of a PS2 room (`R10#5`),
/// `scene` the DS scene the event is bound to (by its text or its place, medium or high). Far places (low) and the chapter's
/// start are left out.
fn run_sets(room: Option<&str>, scene: Option<&str>) -> Vec<RunSet> {
    static T: &str = include_str!("../../../data/ult/run_mode.tsv");
    T.lines()
        .skip(1)
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            if c.len() < 11 {
                return None;
            }
            let fast = match (c[4], c[5]) {
                ("fastrun", _) => true,
                ("runmode", "0") => false,
                _ => return None,
            };
            let hit = match c[8].strip_prefix("вход в комнату ") {
                Some(r) => room == Some(r.trim_end_matches(" ?")),
                None => scene == Some(c[7]) && matches!(c[10], "high" | "medium"),
            };
            hit.then_some((fast, c[6] == "1"))
        })
        .collect()
}

fn apply_run_sets(sets: &[RunSet], why: &str) {
    for &(fast, on) in sets {
        (if fast { &FAST_LET } else { &RUN_MODE }).store(on, Relaxed);
    }
    if !sets.is_empty() {
        eprintln!("ps2room: {} sets the fast run {} and the run mode {}", why, FAST_LET.load(Relaxed), RUN_MODE.load(Relaxed));
    }
}

/// Switches the walk on for the area `area` (index in `scenes/<scene>.json`'s `scene3d`) of the export at `root`.
pub fn enable(root: &str, scene: &str, area: usize) -> Result<(), String> {
    let data = Export::open(root).and_then(|e| e.scene(scene)).map_err(|e| e.to_string())?;
    let index = area;
    let mut area = data.areas.into_iter().nth(area).ok_or("no such area")?;
    // the walls and floors a new game has on (zones.tsv); the events that switch them later are not followed
    let zones = std::fs::read_to_string(std::path::Path::new(root).join("zones.tsv")).map_err(|e| format!("zones.tsv: {}", e))?;
    sunpetal_data::ps2::walk::apply_initial_zones(&mut area, &zones, scene, index);
    // the room's init event sets the run modes on entering it
    apply_run_sets(&run_sets(Some(&format!("{}#{}", scene, index)), None), &format!("{} {}", scene, index));
    *WALK.lock().unwrap() = Some(Walk { area, room: (scene.to_string(), index), last: None, written: None, walker: Walker::default(), inside: Vec::new(), floor: None });
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
    switch(&mut w.area, kind, zone, on).ok_or_else(|| format!("the room has no {} zone {} (kind is hit, height or event)", kind, zone))
}

/// One zone of the area on or off (an event zone: its save bit 0); `None` if there is no such zone.
fn switch(area: &mut Area3d, kind: &str, zone: usize, on: bool) -> Option<()> {
    match kind {
        "hit" => area.hit.get_mut(zone).map(|h| h.enabled = on),
        "height" => area.height.get_mut(zone).map(|t| t.enabled = on),
        "event" => area.events.get_mut(zone).map(|e| e.enabled = on),
        _ => None,
    }
}

/// The zones a PS2 event switches when the hero walks into its automatic event zone (`data/ult/zone_trigger.tsv`, from
/// `tools/ult/event_match.py`; for now the lighthouse switch R11 #39/#40, `sunpetal-ee` F-272): (kind, zone, on).
fn triggers(scene: &str, index: usize, event: i32) -> Vec<(&'static str, usize, bool)> {
    static T: &str = include_str!("../../../data/ult/zone_trigger.tsv");
    T.lines()
        .skip(1)
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            (c.len() == 6 && c[0] == scene && c[1].parse() == Ok(index) && c[2].parse() == Ok(event)).then(|| Some((c[3], c[4].parse().ok()?, c[5] == "on")))?
        })
        .collect()
}

/// The hotel's stairs: on DS one map (A30) for all floors, the floor by the hero's height, `1 + round(y / 6.2)` (`sunpetal-ee` F-277:
/// the `CHANGEMAP 30` targets stand at 0, 6.2, 12.4, 18.6, 25; 5 is the roof's landing). PS2 keeps the floor in `var 0x6002[0]`.
pub const STAIRS_MAP: i32 = 30;
pub fn stairs_floor(y: i32) -> u32 {
    (1.0 + (y as f64 / 4096.0 / 6.2).round()).max(1.0) as u32
}

/// The zones of a stairs' room on a floor (`data/ult/stairs_floor.tsv`, `tools/ult/stairs_floor.py`: R03/R04 by `sunpetal-ee` Y-56e,
/// live, all kinds; the rest by Y-56d, walls and floors): (kind, zone, on). A room that is not that floor's flight gives none.
/// Camera zones are in the table but not followed yet (the stairs' camera is of fixed points, Y-63).
fn stairs_zones(scene: &str, index: usize, floor: u32) -> Vec<(&'static str, usize, bool)> {
    static T: &str = include_str!("../../../data/ult/stairs_floor.tsv");
    T.lines()
        .skip(1)
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            (c.len() >= 6 && c[0] == scene && c[1].parse() == Ok(index) && c[4].parse() == Ok(floor)).then(|| Some((c[2], c[3].parse().ok()?, c[5] == "1")))?
        })
        .collect()
}

/// On the stairs' map, the zones of the walking room follow the hero's floor (set again when the floor changes).
fn follow_floor(w: &mut Walk, map: Option<i32>, y: i32) {
    if map != Some(STAIRS_MAP) {
        w.floor = None;
        return;
    }
    let floor = stairs_floor(y);
    if w.floor == Some(floor) {
        return;
    }
    w.floor = Some(floor);
    let t = stairs_zones(&w.room.0, w.room.1, floor);
    for &(kind, z, on) in &t {
        switch(&mut w.area, kind, z, on);
    }
    eprintln!("ps2room: stairs floor {} in {} {}: {} switches", floor, w.room.0, w.room.1, t.len());
}

/// The hero at `p` (x, z, export units) facing `face`: the automatic event zones walked into this frame fire their switches.
fn fire_events(w: &mut Walk, p: [f32; 2], face: f32) {
    let now = auto_events(&w.area, p, face);
    for &(zone, event) in now.iter().filter(|(z, _)| !w.inside.contains(z)) {
        let t = triggers(&w.room.0, w.room.1, event);
        for &(kind, z, on) in &t {
            switch(&mut w.area, kind, z, on);
        }
        if !t.is_empty() {
            eprintln!("ps2room: event zone {} of {} {} fires #{}: {} switches", zone, w.room.0, w.room.1, event, t.len());
        }
    }
    w.inside = now.into_iter().map(|(z, _)| z).collect();
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
    apply_run_sets(&run_sets(None, Some(ds_scene)), ds_scene);
    let mut g = WALK.lock().unwrap();
    let Some(w) = g.as_mut() else { return 0 };
    let mut n = 0;
    for (scene, index, kind, zone, on) in switches(ds_scene) {
        if w.room.0 == scene && w.room.1 == index {
            switch(&mut w.area, kind, zone, on);
            n += 1;
            eprintln!("ps2room: {} switches {} {} {} of {} {}", ds_scene, kind, zone, if on { "on" } else { "off" }, scene, index);
        }
    }
    n
}

/// The walking room and which of its walls, floors and event zones are on (for the debugger): (scene, index, hit, height, event flags).
pub type ZoneState = (String, usize, Vec<bool>, Vec<bool>, Vec<bool>);
pub fn zone_state() -> Option<ZoneState> {
    let g = WALK.lock().unwrap();
    let w = g.as_ref()?;
    Some((
        w.room.0.clone(),
        w.room.1,
        w.area.hit.iter().map(|h| h.enabled).collect(),
        w.area.height.iter().map(|t| t.enabled).collect(),
        w.area.events.iter().map(|e| e.enabled).collect(),
    ))
}

/// The fast run allowed or not (`walk.md` §3a); its zones that forbid it stay as the room has them.
pub fn set_fast_mode(on: bool) -> Result<(), String> {
    FAST_LET.store(on, Relaxed);
    RUN_MODE.store(on, Relaxed);
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
        let fast = FAST_LET.load(Relaxed) && RUN_MODE.load(Relaxed) && fast_run_allowed(&w.area, from);
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
        let at = w.last.unwrap_or(from);
        let face = w.walker.face;
        fire_events(w, [at[0], at[2]], face);
        follow_floor(w, session::hero_map(), session::hero_pos().map_or(pos[1], |p| p[1]));
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
        // the stairs by the floor (Y-56d): 2F->3F writes the stairs' room only; a stair event bound by place to a story scene is out
        assert!(switches("A04_18").contains(&("R04", 5, "height", 0, false)));
        assert!(!switches("C03_03").iter().any(|z| (z.0, z.1, z.2, z.3) == ("R03", 3, "height", 0)));
    }

    #[test]
    fn the_lighthouse_switch_fires_by_the_hero_place() {
        // R11 room 5 (F-272): #39 turns the spiral's event zones 6..9 off and #40 back on; a wall one turns on, the other off
        let (a, b) = (triggers("R11", 5, 39), triggers("R11", 5, 40));
        assert_eq!((a.len(), b.len()), (49, 49));
        assert!(a.contains(&("event", 6, false)) && b.contains(&("event", 6, true)));
        assert!(a.iter().all(|&(k, z, on)| b.contains(&(k, z, !on))), "#40 undoes #39");
        assert!(triggers("R11", 7, 39).is_empty() && triggers("R03", 5, 39).is_empty());
    }

    #[test]
    fn the_stairs_floor_goes_by_the_height_and_gives_the_flight_zones() {
        // F-277: the CHANGEMAP 30 targets 0 / 6.2 / 12.4 / 18.6 / 25 are floors 1..5
        let fl = |y: f64| stairs_floor((y * 4096.0) as i32);
        assert_eq!([fl(0.0), fl(6.2), fl(12.4), fl(18.6), fl(25.0), fl(-1.0)], [1, 2, 3, 4, 5, 1]);
        // R03's flight is room 3 (Y-56d): each floor of it sets its zones, another room none
        assert!((3..=5).all(|f| !stairs_zones("R03", 3, f).is_empty()));
        assert!(stairs_zones("R03", 0, 3).is_empty());
        let (up, down) = (stairs_zones("R03", 3, 4), stairs_zones("R03", 3, 3));
        assert!(up.iter().any(|z| down.iter().any(|d| (d.0, d.1) == (z.0, z.1) && d.2 != z.2)), "floors 3 and 4 differ");
        // Y-56e: R04's upper flight (room 5) is floors 3..5 by #9/#10, the lower (room 6) floor 2 by #24; all four kinds
        assert!(stairs_zones("R04", 6, 4).is_empty() && stairs_zones("R04", 5, 2).is_empty());
        assert!(!stairs_zones("R04", 6, 2).is_empty() && stairs_zones("R04", 5, 4).iter().any(|z| z.0 == "event"));
    }

    #[test]
    fn a_room_and_a_scene_give_their_run_mode_switches() {
        // OP room 9 (A19 in chapter 0): the run mode on, the fast run off (event 28, pc 313 then 321)
        assert_eq!(run_sets(Some("OP#9"), None), vec![(false, true), (true, false)]);
        // C00_01: text-near, after line 01023
        assert!(run_sets(None, Some("C00_01")).contains(&(true, true)));
        // a far place (low) is left out; another room's chara is not the hero
        assert!(run_sets(None, Some("C13_boat")).is_empty());
        assert!(run_sets(Some("no#0"), None).is_empty());
    }
}
