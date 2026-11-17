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
}

impl RoomEvent {
    /// Whether the PS2-unit place (x, z) is inside the zone (the edge counts in).
    pub fn holds(&self, x: f32, z: f32) -> bool {
        x >= self.x[0] && x <= self.x[1] && z >= self.z[0] && z <= self.z[1]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_and_the_keys_read() {
        let t = parse(r#"[{"model":"m","bone":2,"map":44,"x":[-1,1],"z":[-2,2],"delay":90,"motion":"abc","step":3}]"#).unwrap();
        assert_eq!(t[0].step, Some(3));
        assert_eq!(t[0].bone, 2);
        assert!(t[0].holds(0.0, 2.0) && !t[0].holds(2.0, 0.0));
        let k = motion_keys(r#"{"joints":[{"keys":[{"frame":0,"pos":[0,0,0],"rot":[0,0,0]},{"frame":2,"pos":[1,2,3],"rot":[0,0,1.5]}]}]}"#).unwrap();
        assert_eq!(k.len(), 3);
        // frame 1 holds frame 0
        assert_eq!(k[1].0, [0.0; 3]);
        assert_eq!(k[2].1[2], 1.5);
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
