//! Events of a replaced room that move its objects (K-237, the painting that falls when the hero comes in): the table
//! `data/ult/room_events.json` (`SUNPETAL_ROOM_EVENTS` names another), the hero's place decides, the motion's keys come from the
//! `sunpetal-ee` export (`SUNPETAL_EE_EXPORT`), and the pose goes to the registry the replacement model's drawing reads (`gx.rs`).
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use sunpetal_data::ps2::roomevent::{self, RoomEvent};

/// The table, read once; empty when there is no file.
pub fn table() -> &'static [RoomEvent] {
    static T: OnceLock<Vec<RoomEvent>> = OnceLock::new();
    T.get_or_init(|| {
        let path = std::env::var_os("SUNPETAL_ROOM_EVENTS").map(std::path::PathBuf::from).unwrap_or_else(|| "data/ult/room_events.json".into());
        match std::fs::read_to_string(&path) {
            Ok(t) => roomevent::parse(&t).unwrap_or_else(|e| {
                eprintln!("room events: {}: {}", path.display(), e);
                Vec::new()
            }),
            Err(_) => Vec::new(),
        }
    })
}

/// The motions of the data files the app named (their `MOTN` part), of `SUNPETAL_ULT_MODELS` (a data file, or a folder with
/// `motions.bin`), read once (K-244). A motion the export has (`SUNPETAL_EE_EXPORT`) and these do not is read from it.
#[cfg(feature = "debug-tools")]
fn packed(sha: &str) -> Option<roomevent::Keys> {
    static M: OnceLock<HashMap<String, roomevent::Keys>> = OnceLock::new();
    M.get_or_init(|| {
        let mut all = HashMap::new();
        let mut paths: Vec<std::path::PathBuf> = std::env::var_os("SUNPETAL_ULT_MODELS").map(std::path::PathBuf::from).into_iter().collect();
        paths.extend(crate::nds::model_files());
        for p in paths {
            let bytes = if p.is_dir() {
                std::fs::read(p.join("motions.bin")).ok()
            } else {
                sunpetal_data::container::Container::read_only(&p, |t| t == roomevent::TAG).ok().and_then(|c| c.get(roomevent::TAG).map(|s| s.data.clone()))
            };
            if let Some(m) = bytes.and_then(|b| roomevent::read_motions(&b).map_err(|e| eprintln!("room motions: {}: {}", p.display(), e)).ok()) {
                all.extend(m);
            }
        }
        all
    })
    .get(sha)
    .cloned()
}

/// The game's own `STAGEANIMATION` commands (the handler's entry, `docs/native-script-dispatch.csv`), counted since its script began:
/// the Nth of a scene starts the event whose `call` is N (K-245, the second clip of a shelter).
#[cfg(feature = "debug-tools")]
mod calls {
    use std::cell::Cell;
    thread_local! {
        static N: Cell<u32> = const { Cell::new(0) };
        static ON: Cell<bool> = const { Cell::new(false) };
    }
    const HANDLER: u32 = 0x0204_9439;

    pub fn install() {
        if !ON.with(|o| o.replace(true)) {
            crate::nds::debug::watch(9, HANDLER, |_| N.with(|n| n.set(n.get() + 1)));
        }
    }
    pub fn count() -> u32 {
        N.with(Cell::get)
    }
    pub fn reset() {
        N.with(|n| n.set(0));
    }
}

/// The bones of `model` some event moves: their vertices are kept apart, so that they can be drawn in any pose.
pub fn animated_bones(model: &str) -> Vec<u16> {
    let mut v: Vec<u16> = table().iter().filter(|e| e.model == model).map(|e| e.bone).collect();
    v.sort_unstable();
    v.dedup();
    v
}

type Pose = [f32; 6];
type Poses = HashMap<(String, u16), Pose>;
static POSE: Mutex<Option<Poses>> = Mutex::new(None);

/// The bone's pose now (place x y z, turn x y z: what replaces its rest), if an event holds it.
pub fn pose(model: &str, bone: u16) -> Option<Pose> {
    POSE.lock().unwrap().as_ref()?.get(&(model.to_string(), bone)).copied()
}

#[cfg(feature = "debug-tools")]
fn set_pose(model: &str, bone: u16, p: Pose) {
    POSE.lock().unwrap().get_or_insert_with(HashMap::new).insert((model.to_string(), bone), p);
}

/// Where an event is: not started, waiting for the game's delay, playing a frame of its motion, done (the last pose holds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage {
    Idle,
    Waiting(u32),
    Playing(u32),
    Done,
}

/// One frame of an event's stage: `inside` is whether the hero is in its zone now; `frames` the motion's length.
pub fn step(stage: Stage, inside: bool, delay: u32, frames: u32) -> Stage {
    match stage {
        Stage::Idle if inside => Stage::Waiting(delay),
        Stage::Waiting(0) => Stage::Playing(0),
        Stage::Waiting(n) => Stage::Waiting(n - 1),
        Stage::Playing(f) if f + 1 >= frames => Stage::Done,
        Stage::Playing(f) => Stage::Playing(f + 1),
        s => s,
    }
}

/// `step` with the story: `story` is the chapter's step against the event's (`None` if the event has none). Before its
/// step the object is at rest (a new game takes it back); past it the event has been (a loaded save finds it done).
pub fn step_in_story(story: Option<std::cmp::Ordering>, stage: Stage, inside: bool, delay: u32, frames: u32) -> Stage {
    use std::cmp::Ordering::*;
    match (story, stage) {
        (Some(Less), _) => Stage::Idle,
        // a save loaded past the step finds it done; an event already waiting or playing goes on (the script moves the
        // step on while the painting is still falling)
        (Some(Greater), Stage::Idle) => Stage::Done,
        (Some(Greater), st) => step(st, false, delay, frames),
        (_, st) => step(st, inside, delay, frames),
    }
}

#[cfg(feature = "debug-tools")]
type Keys = roomevent::Keys;

/// Per event: its stage and its motion's keys (read at the first frame; none if the export is missing).
#[cfg(feature = "debug-tools")]
struct State {
    stage: Stage,
    keys: Option<Keys>,
    read: bool,
    /// Whether a script was running at the last frame (an event starts on the edge).
    was_running: bool,
}

#[cfg(feature = "debug-tools")]
static STATES: Mutex<Vec<State>> = Mutex::new(Vec::new());

/// One frame of the room events, from the live game: the hero's map and place, the stages, the poses.
/// `rest_of(model, bone)` gives the bone's rest place (what the pose's move is added to).
#[cfg(feature = "debug-tools")]
pub fn frame(rest_of: impl Fn(&str, u16) -> Option<[f32; 3]>) {
    let t = table();
    if t.is_empty() {
        return;
    }
    let (Some(map), Some(pos)) = (crate::session::hero_map(), crate::session::hero_pos()) else { return };
    // the scene the game read last: the one whose script has just started, at a script's start
    let scene = crate::nds::script_scene();
    calls::install();
    let mut states = STATES.lock().unwrap();
    while states.len() < t.len() {
        states.push(State { stage: Stage::Idle, keys: None, read: false, was_running: false });
    }
    // the DS place in 1/4096 units is the PS2's scene units over ten (this room's shift is none, K-237)
    let (x, z) = (pos[0] as f32 / 4096.0 * 10.0, pos[2] as f32 / 4096.0 * 10.0);
    // the count of the scene's commands starts again with each script
    static LAST: Mutex<(Option<String>, u32)> = Mutex::new((None, 0));
    let (before, now) = {
        let mut last = LAST.lock().unwrap();
        if last.0 != scene {
            last.0 = scene.clone();
            calls::reset();
            last.1 = 0;
        }
        let (before, now) = (last.1, calls::count());
        last.1 = now;
        (before, now)
    };
    for (e, s) in t.iter().zip(states.iter_mut()) {
        if !s.read {
            s.read = true;
            s.keys = packed(&e.motion);
            if s.keys.is_none() {
                if let Some(root) = std::env::var_os("SUNPETAL_EE_EXPORT") {
                    let path = std::path::Path::new(&root).join("motions").join(format!("{}.json", e.motion));
                    s.keys = std::fs::read_to_string(path).ok().and_then(|t| roomevent::motion_keys(&t).ok());
                }
            }
        }
        let Some(keys) = s.keys.as_ref().filter(|k| !k.is_empty()) else { continue };
        // a running script holds -1 or a wait's count (a few hundred at most); idle is -2 (`nds::script_idle`)
        let running = crate::nds::script_mark().is_some_and(|w| w == u32::MAX || w <= 0x1000);
        let started = running && !s.was_running;
        s.was_running = running;
        // a named scene decides alone: the script is the DS game's own trigger, and the zone and the map are the PS2's (the room
        // may stand shifted from the DS one, the door of A45 does by some 180 units); the model is drawn only where it stands
        let inside = match (&e.scene, e.on_script) {
            (Some(s), true) if e.call > 1 => scene.as_ref() == Some(s) && before < e.call && e.call <= now,
            (Some(s), true) => started && scene.as_ref() == Some(s),
            (None, true) => started && map == e.map && e.holds_within(x, z, 6.0),
            _ => map == e.map && e.holds(x, z),
        };
        // the story decides first: before the event's step it is not played (a new game takes the object back), past it it has been
        let story = e.step.and_then(|n| crate::session::story_value(0).map(|v| v.cmp(&n)));
        s.stage = step_in_story(story, s.stage, inside, e.delay, keys.len() as u32);
        let at = match s.stage {
            Stage::Playing(f) => Some(f as usize),
            Stage::Done => Some(keys.len() - 1),
            _ => None,
        };
        if let (Some(f), Some(rest)) = (at, rest_of(&e.model, e.bone)) {
            let (p, r) = keys[f];
            set_pose(&e.model, e.bone, [rest[0] + p[0], rest[1] + p[1], rest[2] + p[2], r[0], r[1], r[2]]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_waits_plays_every_frame_and_holds() {
        let mut s = Stage::Idle;
        for _ in 0..5 {
            s = step(s, false, 2, 3);
        }
        assert_eq!(s, Stage::Idle);
        s = step(s, true, 2, 3);
        assert_eq!(s, Stage::Waiting(2));
        // the hero leaves: it goes on
        let mut seen = vec![];
        for _ in 0..7 {
            s = step(s, false, 2, 3);
            seen.push(s);
        }
        assert_eq!(seen, [Stage::Waiting(1), Stage::Waiting(0), Stage::Playing(0), Stage::Playing(1), Stage::Playing(2), Stage::Done, Stage::Done]);
    }

    #[test]
    fn the_story_step_decides_before_the_zone() {
        use std::cmp::Ordering::*;
        // before the step: at rest, even inside the zone; past it: done at once, a save loaded after the event
        assert_eq!(step_in_story(Some(Less), Stage::Done, true, 5, 9), Stage::Idle);
        assert_eq!(step_in_story(Some(Greater), Stage::Idle, false, 5, 9), Stage::Done);
        // at the step: the zone starts it; the script moving the step on does not stop a running event
        assert_eq!(step_in_story(Some(Equal), Stage::Idle, true, 5, 9), Stage::Waiting(5));
        assert_eq!(step_in_story(Some(Greater), Stage::Playing(3), true, 5, 9), Stage::Playing(4));
        assert_eq!(step_in_story(Some(Greater), Stage::Waiting(3), true, 5, 9), Stage::Waiting(2));
        assert_eq!(step_in_story(None, Stage::Idle, true, 5, 9), Stage::Waiting(5));
    }
}
