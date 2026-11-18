//! Events of a PS2 room that move one of its objects (`docs/NATIVE-ENGINE-PLAN.md`; K-237, the falling painting): where the
//! hero must stand, how long the game waits, which motion plays and which bone of the replacement model it moves. The table is data
//! (`data/ult/room_events.json`), the motion's keys come from the `sunpetal-ee` export (`motions/<sha>.json`).
use serde_json::Value;

/// One event: the hero on `map` inside the rectangle (x, z in PS2 scene units) starts it; after `delay` frames `motion` plays on `bone`.
#[derive(Clone, Debug, PartialEq)]
pub struct RoomEvent {
    /// The DS model the replacement stands for (`SPM2` bind) and its bone the motion moves.
    pub model: String,
    pub bone: u16,
    /// The DS map the hero is on.
    pub map: i32,
    pub x: [f32; 2],
    pub z: [f32; 2],
    pub delay: u32,
    pub motion: String,
    /// The story step (`SETVALUE 0 n`) at which the DS game plays the event: before it the object is at rest, after it already moved.
    pub step: Option<i32>,
    /// Starts when the DS game starts a script on this map at this step (the zone only has to hold, widened by `margin`),
    /// not when the hero enters the zone: the DS event is its own, and fires where ours may not reach (K-237).
    pub on_script: bool,
    /// The DS scene whose script starts it (with `on_script`): another script on the map does not (K-242); none: any.
    pub scene: Option<String>,
    /// With `scene`: the script's own call of `STAGEANIMATION` (1, 2, ...) that starts it; the first starts with the script (K-245).
    pub call: u32,
    /// The object is hidden this many frames after its motion starts (PS2 `SetObjVisibleScene3D(0, 0)`: C13_06's switch, K-245).
    pub hide_after: Option<u32>,
}

impl RoomEvent {
    /// Whether the PS2-unit place (x, z) is inside the zone (the edge counts in).
    pub fn holds(&self, x: f32, z: f32) -> bool {
        self.holds_within(x, z, 0.0)
    }

    /// The same with the zone widened by `margin` on every side.
    pub fn holds_within(&self, x: f32, z: f32, margin: f32) -> bool {
        x >= self.x[0] - margin && x <= self.x[1] + margin && z >= self.z[0] - margin && z <= self.z[1] + margin
    }
}

fn pair(v: &Value, k: &str) -> Result<[f32; 2], String> {
    let a = v.get(k).and_then(Value::as_array).filter(|a| a.len() == 2).ok_or(format!("no pair `{}`", k))?;
    Ok([a[0].as_f64().ok_or("bad number")? as f32, a[1].as_f64().ok_or("bad number")? as f32])
}

/// The table from its JSON text: a list of events.
pub fn parse(text: &str) -> Result<Vec<RoomEvent>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    v.as_array()
        .ok_or("the table is a list")?
        .iter()
        .map(|e| {
            Ok(RoomEvent {
                model: e.get("model").and_then(Value::as_str).ok_or("no `model`")?.to_string(),
                bone: e.get("bone").and_then(Value::as_u64).ok_or("no `bone`")? as u16,
                map: e.get("map").and_then(Value::as_i64).ok_or("no `map`")? as i32,
                x: pair(e, "x")?,
                z: pair(e, "z")?,
                delay: e.get("delay").and_then(Value::as_u64).unwrap_or(0) as u32,
                motion: e.get("motion").and_then(Value::as_str).ok_or("no `motion`")?.to_string(),
                step: e.get("step").and_then(Value::as_i64).map(|s| s as i32),
                on_script: e.get("on_script").and_then(Value::as_bool).unwrap_or(false),
                scene: e.get("scene").and_then(Value::as_str).map(str::to_string),
                hide_after: e.get("hide_after").and_then(Value::as_u64).map(|n| n as u32),
                call: e.get("call").and_then(Value::as_u64).unwrap_or(1) as u32,
            })
        })
        .collect()
}

/// The keys of a single-joint motion (`motions/<sha>.json`): per frame the move and the turn (x y z, radians) from the joint's rest.
pub fn motion_keys(text: &str) -> Result<Vec<([f32; 3], [f32; 3])>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let keys = v.get("joints").and_then(Value::as_array).and_then(|j| j.first()).and_then(|j| j.get("keys")).and_then(Value::as_array).ok_or("no joint keys")?;
    let triple = |k: &Value, n: &str| -> Result<[f32; 3], String> {
        let a = k.get(n).and_then(Value::as_array).filter(|a| a.len() == 3).ok_or(format!("no `{}`", n))?;
        Ok([0, 1, 2].map(|i| a[i].as_f64().unwrap_or(0.0) as f32))
    };
    let mut out: Vec<(usize, ([f32; 3], [f32; 3]))> =
        keys.iter().map(|k| Ok((k.get("frame").and_then(Value::as_u64).ok_or("no `frame`")? as usize, (triple(k, "pos")?, triple(k, "rot")?)))).collect::<Result<_, String>>()?;
    out.sort_by_key(|k| k.0);
    // one key per frame: a missing frame holds the key before it
    let last = out.last().map_or(0, |k| k.0);
    let mut dense = Vec::with_capacity(last + 1);
    let mut it = out.into_iter().peekable();
    let mut cur = ([0.0; 3], [0.0; 3]);
    for f in 0..=last {
        while it.peek().is_some_and(|k| k.0 <= f) {
            cur = it.next().unwrap().1;
        }
        dense.push(cur);
    }
    Ok(dense)
}

/// The data file's part of room motions (K-244): the keys of the motions the table names, so that the game needs no export.
pub const TAG: &[u8; 4] = b"MOTN";

/// Dense keys of one motion (what `motion_keys` gives).
pub type Keys = Vec<([f32; 3], [f32; 3])>;

/// The part: `sunpetal-ee`'s `SPM1` motion pack (`notes/formats/motionpack.md`), one joint per motion.
/// `range` is the frame count (the game holds the last key after it).
pub fn pack_motions(motions: &std::collections::BTreeMap<String, Keys>) -> Vec<u8> {
    let mut o = b"SPM1".to_vec();
    o.extend_from_slice(&(motions.len() as u32).to_le_bytes());
    for (sha, keys) in motions {
        let mut name = sha.as_bytes().to_vec();
        name.resize(10, b'0');
        o.extend_from_slice(&name);
        o.extend_from_slice(&(keys.len() as u32).to_le_bytes());
        o.extend_from_slice(&(keys.len() as u32).to_le_bytes());
        o.extend_from_slice(&1u16.to_le_bytes());
        o.extend_from_slice(&0u16.to_le_bytes());
        for (p, r) in keys {
            for v in p.iter().chain(r) {
                o.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    o
}

/// The motions of a part (or of `motions.bin` of `sunpetal-ee`, the same format); of several joints the first is taken.
pub fn read_motions(b: &[u8]) -> Result<std::collections::HashMap<String, Keys>, String> {
    if b.get(..4) != Some(b"SPM1") {
        return Err("not a motion pack".into());
    }
    let at = |o: usize, n: usize| b.get(o..o + n).ok_or_else(|| "motion pack is cut short".to_string());
    let u32at = |o: usize| at(o, 4).map(|x| u32::from_le_bytes(x.try_into().unwrap()));
    let f32at = |o: usize| at(o, 4).map(|x| f32::from_le_bytes(x.try_into().unwrap()));
    let (count, mut o, mut out) = (u32at(4)? as usize, 8, std::collections::HashMap::new());
    for _ in 0..count {
        let sha = String::from_utf8_lossy(at(o, 10)?).into_owned();
        let frames = u32at(o + 14)? as usize;
        let joints = u16::from_le_bytes(at(o + 18, 2)?.try_into().unwrap()) as usize;
        o += 22;
        let mut keys = Vec::with_capacity(frames);
        for j in 0..joints {
            for _ in 0..frames {
                if j == 0 {
                    let v = [0, 1, 2, 3, 4, 5].map(|i| f32at(o + 4 * i));
                    let g = |i: usize| v[i].clone();
                    keys.push(([g(0)?, g(1)?, g(2)?], [g(3)?, g(4)?, g(5)?]));
                }
                o += 24;
            }
        }
        out.insert(sha, keys);
    }
    if o != b.len() {
        return Err("motion pack has bytes left over".into());
    }
    Ok(out)
}

/// The motions the table names, read from an export's `motions/<sha>.json`.
pub fn collect_motions(table: &[RoomEvent], export: &std::path::Path) -> Result<std::collections::BTreeMap<String, Keys>, String> {
    let mut out = std::collections::BTreeMap::new();
    for e in table {
        if !out.contains_key(&e.motion) {
            let path = export.join("motions").join(format!("{}.json", e.motion));
            let text = std::fs::read_to_string(&path).map_err(|x| format!("{}: {}", path.display(), x))?;
            out.insert(e.motion.clone(), motion_keys(&text).map_err(|x| format!("{}: {}", path.display(), x))?);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_and_the_keys_read() {
        let t = parse(r#"[{"model":"m","bone":2,"map":44,"x":[-1,1],"z":[-2,2],"delay":90,"motion":"abc","step":3,"on_script":true,"scene":"C01_25"}]"#).unwrap();
        assert_eq!(t[0].scene.as_deref(), Some("C01_25"));
        assert!(t[0].on_script && t[0].holds_within(1.5, 0.0, 1.0) && !t[0].holds(1.5, 0.0));
        assert_eq!(t[0].step, Some(3));
        assert_eq!(t[0].bone, 2);
        assert_eq!((t[0].call, t[0].hide_after), (1, None));
        let h = parse(r#"[{"model":"m","bone":0,"map":1,"x":[0,1],"z":[0,1],"motion":"a","call":2,"hide_after":60}]"#).unwrap();
        assert_eq!((h[0].call, h[0].hide_after), (2, Some(60)));
        assert!(t[0].holds(0.0, 2.0) && !t[0].holds(2.0, 0.0));
        let k = motion_keys(r#"{"joints":[{"keys":[{"frame":0,"pos":[0,0,0],"rot":[0,0,0]},{"frame":2,"pos":[1,2,3],"rot":[0,0,1.5]}]}]}"#).unwrap();
        assert_eq!(k.len(), 3);
        // frame 1 holds frame 0
        assert_eq!(k[1].0, [0.0; 3]);
        assert_eq!(k[2].1[2], 1.5);
    }

    #[test]
    fn the_motion_part_round_trips() {
        let mut m = std::collections::BTreeMap::new();
        m.insert("85bcbcd4a3".to_string(), vec![([0.0; 3], [0.0; 3]), ([1.0, 2.0, 3.0], [0.0, 0.0, 1.5])]);
        m.insert("ab".to_string(), vec![([0.5; 3], [0.25; 3])]);
        let b = pack_motions(&m);
        let r = read_motions(&b).unwrap();
        assert_eq!(r["85bcbcd4a3"], m["85bcbcd4a3"]);
        assert_eq!(r.len(), 2);
        assert!(read_motions(&b[..b.len() - 1]).is_err());
    }

    /// The motions of the real table: our pack carries every one of them, frame for frame like `sunpetal-ee`'s `motions.bin` (Y-48).
    #[test]
    fn the_real_table_packs_like_the_ee_motion_pack() {
        let Some(root) = std::env::var_os("SUNPETAL_EE_EXPORT") else { return };
        let root = std::path::PathBuf::from(root);
        let Ok(text) = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/ult/room_events.json")) else { return };
        let table = parse(&text).unwrap();
        let mine = read_motions(&pack_motions(&collect_motions(&table, &root).unwrap())).unwrap();
        let Ok(theirs) = std::fs::read(root.join("../../out/y48/motions.bin")) else { return };
        let theirs = read_motions(&theirs).unwrap();
        for (sha, keys) in &mine {
            assert_eq!(theirs.get(sha), Some(keys), "{}", sha);
        }
    }

    /// The painting of R01 (`sunpetal-ee` F-233): 61 frames, ends turned a quarter about z.
    #[test]
    fn the_real_painting_motion_has_61_frames() {
        let Some(root) = std::env::var_os("SUNPETAL_EE_EXPORT") else { return };
        let Ok(text) = std::fs::read_to_string(std::path::Path::new(&root).join("motions/85bcbcd4a3.json")) else { return };
        let k = motion_keys(&text).unwrap();
        assert_eq!(k.len(), 61);
        assert!((k[60].1[2] - std::f32::consts::FRAC_PI_2).abs() < 1e-3);
    }
}
