//! Walking of the PS2 hero as the game does it (K-236; `sunpetal-ee/notes/formats/walk.md`, `hit.md`): the stick, the
//! step relative to the camera, the turn, the walls and the floor. Pure functions of the exported collision
//! (`export::Area3d`); the units are the export's (tenths of the DS world's).
use super::export::{Area3d, HitShape};

/// Sets which walls and floors are on as a new game has them (`zones.tsv` of the export, `hit.md` §7): the file's own
/// `enabled` is 0 everywhere, the state is the save's. `scene3d` is the room's index. Zones the table does not name stay off.
pub fn apply_initial_zones(area: &mut Area3d, zones_tsv: &str, scene: &str, scene3d: usize) {
    for h in &mut area.hit {
        h.enabled = false;
    }
    for t in &mut area.height {
        t.enabled = false;
    }
    let mut rows = zones_tsv.lines();
    let Some(head) = rows.next() else { return };
    let col = |n: &str| head.split('\t').position(|h| h == n);
    let (Some(cs), Some(c3), Some(ck), Some(cz), Some(ce)) = (col("scene"), col("scene3d"), col("kind"), col("zone"), col("initial_enabled")) else { return };
    for row in rows {
        let c: Vec<&str> = row.split('\t').collect();
        if c.len() <= ce || c[cs] != scene || c[c3].parse() != Ok(scene3d) {
            continue;
        }
        let (Ok(z), on) = (c[cz].parse::<usize>(), c[ce] == "1") else { continue };
        match c[ck] {
            "hit" => area.hit.get_mut(z).iter_mut().for_each(|h| h.enabled = on),
            "height" => area.height.get_mut(z).iter_mut().for_each(|t| t.enabled = on),
            _ => {}
        }
    }
}

/// The processed stick axis of a raw byte `raw` (0..=255, centre 127) given the value it had (`ghmPadAnalogProc`):
/// dead zone 48, gives -128..=127, a change of 2 or less is dropped unless it lands on the rest or an end.
pub fn pad_axis(raw: u8, old: i8) -> i8 {
    let d = raw as i32 - 127;
    let new = if d.abs() <= 48 {
        0
    } else if d > 0 {
        ((d - 48) * 127 / 78).min(127)
    } else {
        ((d + 48) * 129 / 80).max(-128)
    };
    if (old as i32 - new).abs() > 2 || matches!(new, 127 | 0 | -128) {
        new as i8
    } else {
        old
    }
}

/// How the stick moves the hero: walk speed, run (the stick pushed past 120), per frame.
#[derive(Clone, Copy, Debug)]
pub struct Speeds {
    pub walk: f32,
    pub run: f32,
}

/// Seleka's (R01): `GIG+0x20 = 0.1` makes walk 0.25 and run 0.5 a frame (`walk.md` §3).
pub const SELEKA: Speeds = Speeds { walk: 0.25, run: 0.5 };

/// The step the stick `(dx, dy)` makes in a frame, relative to the camera (`angle = atan2(dx, dy) - π - correction`),
/// as `(x, z)`; none when the stick rests.
pub fn stick_step(stick: (i8, i8), camera_correction: f32, s: Speeds) -> [f32; 2] {
    let (dx, dy) = (stick.0 as f32, stick.1 as f32);
    if dx == 0.0 && dy == 0.0 {
        return [0.0; 2];
    }
    let speed = if dx.hypot(dy) > 120.0 { s.run } else { s.walk };
    let angle = dx.atan2(dy) - std::f32::consts::PI - camera_correction;
    [speed * angle.sin(), speed * angle.cos()]
}

/// The direction the hero faces for a stick (0 is +z, π/2 is +x), `SetTargetDirection`.
pub fn stick_direction(stick: (i8, i8), camera_correction: f32) -> Option<f32> {
    (stick != (0, 0)).then(|| (stick.0 as f32).atan2(stick.1 as f32) - std::f32::consts::PI - camera_correction)
}

/// Frames of a turn by `diff` radians (`SetTurnDirection`), and whether the hero stands still for the first one.
pub fn turn_frames(diff: f32) -> (u32, bool) {
    let n = (diff.abs() * 4.0 / std::f32::consts::PI * 9.0) as u32;
    if diff.abs() > std::f32::consts::FRAC_PI_2 {
        (n / 2, true)
    } else {
        (n, false)
    }
}

/// A turn of the face toward a heading (`SetTurnDirection`, `walk.md` §4): the whole difference in `total` equal frames.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Turn {
    start: f32,
    diff: f32,
    total: u32,
    done: u32,
    target: f32,
}

fn wrap(a: f32) -> f32 {
    use std::f32::consts::PI;
    (a + PI).rem_euclid(2.0 * PI) - PI
}

/// One frame of the face: `face` goes toward `target` (radians, 0 is +z, π/2 is +x). A new target starts a turn; a small one
/// (no whole frame) snaps. `turn` keeps the running one.
pub fn face_step(turn: &mut Option<Turn>, face: f32, target: f32) -> f32 {
    if turn.is_none_or(|t| wrap(t.target - target).abs() > 1e-3) {
        let diff = wrap(target - face);
        let (total, _) = turn_frames(diff);
        *turn = Some(Turn { start: face, diff, total, done: 0, target });
    }
    let t = turn.as_mut().unwrap();
    if t.done >= t.total {
        return wrap(t.target);
    }
    t.done += 1;
    wrap(t.start + t.diff * t.done as f32 / t.total as f32)
}

/// Where `p` (x, z) stands against a convex polygon walked in the positive direction (`ContainPointClosedLine`, `hit.md` §8.1):
/// 1 inside, 0 on the edge, -1 outside. Points are (x, z).
fn contain_closed(poly: &[[f32; 2]], p: [f32; 2]) -> i32 {
    let (mut on, mut out) = (false, false);
    for (i, cur) in poly.iter().enumerate() {
        let prev = poly[(i + poly.len() - 1) % poly.len()];
        let c = (cur[0] - prev[0]) * (p[1] - prev[1]) - (cur[1] - prev[1]) * (p[0] - prev[0]);
        on |= c == 0.0;
        out |= c < 0.0;
    }
    if out {
        -1
    } else if on {
        0
    } else {
        1
    }
}

/// `q` projected on the line of edge `i` (`hit.md` §8.2).
fn project_on_edge(poly: &[[f32; 2]], i: usize, q: [f32; 2]) -> [f32; 2] {
    let (cur, prev) = (poly[i], poly[(i + poly.len() - 1) % poly.len()]);
    let (dx, dz) = (cur[0] - prev[0], cur[1] - prev[1]);
    let cross = dx * (q[1] - prev[1]) - dz * (q[0] - prev[0]);
    let l2 = dx * dx + dz * dz;
    [q[0] + dz * cross / l2, q[1] - dx * cross / l2]
}

/// `OutContainPointClosedLine`: `q` pushed out along the nearest edge (least `cross / L`).
fn push_out(poly: &[[f32; 2]], q: [f32; 2]) -> [f32; 2] {
    let mut best: Option<(f32, usize)> = None;
    for (i, cur) in poly.iter().enumerate() {
        let prev = poly[(i + poly.len() - 1) % poly.len()];
        let (dx, dz) = (cur[0] - prev[0], cur[1] - prev[1]);
        let d = (dx * (q[1] - prev[1]) - dz * (q[0] - prev[0])) / (dx * dx + dz * dz).sqrt();
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, i));
        }
    }
    best.map_or(q, |(_, i)| project_on_edge(poly, i, q))
}

/// The step `p` -> `q` against a polygon wall (`CrossLineLine`, `hit.md` §8.3): the end moved if the step hits it.
fn collide_polygon(poly: &[[f32; 2]], p: [f32; 2], q: [f32; 2]) -> [f32; 2] {
    if poly.len() < 3 {
        return q;
    }
    match contain_closed(poly, p) {
        1 => return push_out(poly, q),
        0 => match contain_closed(poly, q) {
            1 => return push_out(poly, q),
            0 => return q,
            _ => {}
        },
        _ => {}
    }
    if q == p {
        return q;
    }
    for (i, cur) in poly.iter().enumerate() {
        let prev = poly[(i + poly.len() - 1) % poly.len()];
        let (dx, dz) = (cur[0] - prev[0], cur[1] - prev[1]);
        let cross = |v: [f32; 2]| dx * (v[1] - prev[1]) - dz * (v[0] - prev[0]);
        let s1 = (p[1] - prev[1]) * (q[0] - p[0]) - (p[0] - prev[0]) * (q[1] - p[1]);
        let s2 = (p[1] - cur[1]) * (q[0] - p[0]) - (p[0] - cur[0]) * (q[1] - p[1]);
        if cross(p) <= 0.0 && cross(q) > 0.0 && s1 <= 0.0 && s2 > 0.0 {
            return project_on_edge(poly, i, q);
        }
    }
    q
}

/// The wall hit: the end `q` of a step from `p` (x, z) kept out of the area's shapes (rectangles, ellipses and polygons;
/// not done: the switching of zones by the game's events): a rectangle gives only the coordinate of the face crossed (the hero slides along it).
pub fn collide(area: &Area3d, p: [f32; 2], q: [f32; 2]) -> [f32; 2] {
    let mut q = q;
    for h in area.hit.iter().filter(|h| h.enabled) {
        match h.shape {
            HitShape::Rect { x0, z0, dx, dz } => {
                let (x1, z1) = (x0 + dx, z0 + dz);
                let inside = |v: [f32; 2]| v[0] >= x0 && v[0] <= x1 && v[1] >= z0 && v[1] <= z1;
                if !inside(q) {
                    continue;
                }
                if inside(p) && !(p[0] == x0 || p[0] == x1 || p[1] == z0 || p[1] == z1) {
                    // inside: pushed to the nearest face
                    let d = [q[0] - x0, x1 - q[0], q[1] - z0, z1 - q[1]];
                    let m = d.iter().cloned().fold(f32::MAX, f32::min);
                    match d.iter().position(|v| *v == m) {
                        Some(0) => q[0] = x0,
                        Some(1) => q[0] = x1,
                        Some(2) => q[1] = z0,
                        _ => q[1] = z1,
                    }
                    continue;
                }
                // from outside: the axis entered last is the face crossed
                let entry = |a: f32, lo: f32, hi: f32, b: f32| -> (f32, f32) {
                    // (parameter of the entry, the face)
                    // a start on the face counts as outside, entering at once
                    if a <= lo && b > lo {
                        ((lo - a) / (b - a), lo)
                    } else if a >= hi && b < hi {
                        ((hi - a) / (b - a), hi)
                    } else {
                        (f32::MIN, a)
                    }
                };
                let (tx, fx) = entry(p[0], x0, x1, q[0]);
                let (tz, fz) = entry(p[1], z0, z1, q[1]);
                if tx >= tz {
                    q[0] = fx;
                } else {
                    q[1] = fz;
                }
            }
            HitShape::Ellipse { cx, cz, rx, rz } => {
                let norm = |v: [f32; 2]| (((v[0] - cx) / rx).powi(2) + ((v[1] - cz) / rz).powi(2)).sqrt();
                let n = norm(q);
                if n <= 1.0 && n > 0.0 {
                    let k = 1.0009177 / n;
                    q = [cx + (q[0] - cx) * k, cz + (q[1] - cz) * k];
                }
            }
            HitShape::Polygon(ref poly) => q = collide_polygon(poly, p, q),
        }
    }
    q
}

/// The floor's height at `(x, z)`: the first triangle that holds the point (`ContainPoint3D`).
pub fn floor_height(area: &Area3d, x: f32, z: f32) -> Option<f32> {
    area.height.iter().filter(|t| t.enabled && t.points.len() == 3).find_map(|t| {
        let (p0, p1, p2) = (t.points[0], t.points[1], t.points[2]);
        let c0 = (z - p0[2]) * (p1[0] - p0[0]) - (x - p0[0]) * (p1[2] - p0[2]);
        let c1 = (z - p1[2]) * (p2[0] - p1[0]) - (x - p1[0]) * (p2[2] - p1[2]);
        let c2 = (z - p2[2]) * (p0[0] - p2[0]) - (x - p2[0]) * (p0[2] - p2[2]);
        if c0 < 0.0 || c1 < 0.0 || c2 < 0.0 {
            return None;
        }
        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let den = e1[2] * e2[0] - e1[0] * e2[2];
        if den == 0.0 {
            return None;
        }
        Some(p0[1] + ((e1[0] * e2[1] - e1[1] * e2[0]) * (p0[2] - z) + (e1[1] * e2[2] - e1[2] * e2[1]) * (p0[0] - x)) / den)
    })
}

/// One frame of the hero's walk on the ground: `pos` (x, y, z) goes by the stick and the walls, y to the floor.
pub fn walk_frame(area: &Area3d, pos: [f32; 3], stick: (i8, i8), camera_correction: f32, s: Speeds) -> [f32; 3] {
    let st = stick_step(stick, camera_correction, s);
    let q = collide(area, [pos[0], pos[2]], [pos[0] + st[0], pos[2] + st[1]]);
    [q[0], floor_height(area, q[0], q[1]).unwrap_or(pos[1]), q[1]]
}

#[cfg(test)]
mod tests {
    use super::super::export::{HeightTri, Hit};
    use super::*;

    fn area(hit: Vec<Hit>) -> Area3d {
        Area3d { index: 0, record: 0, hit, height: vec![] }
    }

    fn rect(x0: f32, z0: f32, dx: f32, dz: f32) -> Hit {
        Hit { enabled: true, shape: HitShape::Rect { x0, z0, dx, dz } }
    }

    #[test]
    fn the_pad_gives_what_the_game_logged() {
        // raw -> processed, `walk.md` §2: (11,215) -> (-109,65), (59,251) -> (-32,123), (39,11) -> (-64,-109)
        for (raw, want) in [(11u8, -109i8), (215, 65), (59, -32), (251, 123), (39, -64), (11, -109)] {
            assert_eq!(pad_axis(raw, 0), want, "raw {}", raw);
        }
        assert_eq!(pad_axis(130, 0), 0);
        assert_eq!(pad_axis(255, 0), 127);
        assert_eq!(pad_axis(0, 0), -127); // (-127 + 48)·129/80, cut toward zero
    }

    #[test]
    fn the_step_is_the_speed_in_the_stick_direction_turned_by_the_camera() {
        let s = stick_step((0, -100), 0.0, SELEKA);
        // atan2(0,-100) - π = 0: along +z
        assert!(s[0].abs() < 1e-6 && (s[1] - 0.25).abs() < 1e-6, "{:?}", s);
        let r = stick_step((0, -127), 0.0, SELEKA);
        assert!((r[1] - 0.5).abs() < 1e-6);
        assert_eq!(stick_step((0, 0), 0.3, SELEKA), [0.0; 2]);
    }

    #[test]
    fn a_wall_stops_one_axis_and_the_other_slides() {
        let a = area(vec![rect(10.0, -5.0, 5.0, 10.0)]);
        // from the left toward +x and a bit of +z: x stays on the face, z goes on
        let q = collide(&a, [9.9, 0.0], [10.2, 0.1]);
        assert_eq!(q, [10.0, 0.1]);
        // free of the wall: unchanged
        assert_eq!(collide(&a, [0.0, 0.0], [0.2, 0.0]), [0.2, 0.0]);
    }

    #[test]
    fn the_floor_is_interpolated() {
        let mut a = area(vec![]);
        a.height.push(HeightTri { enabled: true, points: vec![[0.0, 10.0, 0.0, 1.0], [10.0, 20.0, 0.0, 1.0], [0.0, 30.0, 10.0, 1.0]] });
        let y = floor_height(&a, 2.0, 2.0).unwrap();
        assert!((y - (10.0 + 2.0 + 4.0)).abs() < 1e-4, "{}", y);
        assert_eq!(floor_height(&a, 20.0, 20.0), None);
    }

    #[test]
    fn the_face_turns_in_equal_frames_and_a_small_turn_snaps() {
        let mut t = None;
        // 90 degrees: 18 frames of 5 degrees
        let mut f = 0.0;
        for _ in 0..18 {
            f = face_step(&mut t, f, std::f32::consts::FRAC_PI_2);
        }
        assert!((f - std::f32::consts::FRAC_PI_2).abs() < 1e-5, "{}", f);
        assert!((face_step(&mut t, f, std::f32::consts::FRAC_PI_2) - f).abs() < 1e-5);
        // across the wrap the short way
        let mut t = None;
        let f = face_step(&mut t, 3.0, -3.0);
        assert!(f > 3.0 || f < -3.0, "{}", f);
        // 3 degrees snap
        let mut t = None;
        assert!((face_step(&mut t, 0.0, 0.05) - 0.05).abs() < 1e-6);
    }

    #[test]
    fn a_big_turn_stands_a_frame_and_takes_half_the_frames() {
        assert_eq!(turn_frames(std::f32::consts::PI), (18, true));
        assert_eq!(turn_frames(0.5), (5, false));
    }

    /// The live walk of R01 (`out/y21/probe.json` of sunpetal-ee, three runs of 60 frames): the model from the key and the stick
    /// of a frame gives the key of the next within 1e-3 (walls, the turn's walk speed and the run included).
    #[test]
    fn the_model_follows_the_live_walk() {
        let Some(root) = std::env::var_os("SUNPETAL_EE_EXPORT") else { return };
        let root = std::path::PathBuf::from(root);
        let probe = root.join("../../y21/probe.json");
        let Ok(text) = std::fs::read_to_string(&probe) else { return };
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let scene = super::super::export::Export::open(&root).unwrap().scene("R01").unwrap();
        let mut area = scene.areas[0].clone();
        apply_initial_zones(&mut area, &std::fs::read_to_string(root.join("zones.tsv")).unwrap(), "R01", 0);
        let area = &area;
        for run in ["straight", "diagonal", "wall"] {
            let fr = v[run].as_array().unwrap();
            let f = |i: usize, k: &str| fr[i][k].as_f64().unwrap() as f32;
            let key = |i: usize| [fr[i]["key"][0].as_f64().unwrap() as f32, fr[i]["key"][1].as_f64().unwrap() as f32, fr[i]["key"][2].as_f64().unwrap() as f32];
            let (mut worst, mut n) = (0f32, 0);
            for i in 0..fr.len() - 1 {
                let state = fr[i]["state"].as_i64().unwrap();
                // a pair that ends standing is the stick's release: the frame's own stick is already gone
                if fr[i + 1]["state"].as_i64().unwrap() == 0 {
                    continue;
                }
                let moving = state != 0;
                // the turn holds the hero to walk whatever the stick (`walk.md` §4)
                let speeds = if state == 1 { Speeds { walk: 0.25, run: 0.25 } } else { SELEKA };
                let raw = |k: &str| fr[i][k].as_i64().unwrap() as u8 as i8;
                let want = key(i + 1);
                let got = if moving { walk_frame(area, key(i), (raw("dx"), raw("dy")), f(i, "corr"), speeds) } else { key(i) };
                let e = (got[0] - want[0]).abs().max((got[2] - want[2]).abs());
                if e > 1e-3 {
                    eprintln!("{run} frame {}: got {:?} want {:?} state {}", fr[i]["frame"], got, want, fr[i + 1]["state"]);
                }
                worst = worst.max(e);
                n += 1;
            }
            eprintln!("{run}: {n} frames, worst {worst}");
            assert!(worst < 1e-3, "{run}: {worst}");
        }
    }

    /// The triangle of R01's room 3 against the live walk of Y-28b (`out/y28b/probe.json`, 240 frames, 201 in contact): the end of
    /// each step within 1e-4 of the game's.
    #[test]
    fn the_polygon_wall_follows_the_live_walk() {
        let Some(root) = std::env::var_os("SUNPETAL_EE_EXPORT") else { return };
        let root = std::path::PathBuf::from(root);
        let Ok(text) = std::fs::read_to_string(root.join("../../y28b/probe.json")) else { return };
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let mut area = super::super::export::Export::open(&root).unwrap().scene("R01").unwrap().areas[3].clone();
        apply_initial_zones(&mut area, &std::fs::read_to_string(root.join("zones.tsv")).unwrap(), "R01", 3);
        let (mut worst, mut n) = (0f32, 0);
        for row in v["rows"].as_array().unwrap() {
            let (b, a) = (&row["before"], &row["after"]);
            let g = |o: &serde_json::Value, i: usize| o["key"][i].as_f64().unwrap() as f32;
            let (dx, dy) = (b["dx"].as_i64().unwrap() as f32, b["dy"].as_i64().unwrap() as f32);
            let speed = match b["state"].as_i64().unwrap() {
                1 => 0.25,
                2 => 0.5,
                _ => continue,
            };
            let m = dx.hypot(dy);
            if m == 0.0 {
                continue;
            }
            // a scripted walk (mode 1) goes by the world: speed times the stick's unit direction
            let (p, q) = ([g(b, 0), g(b, 2)], [g(b, 0) + speed * dx / m, g(b, 2) + speed * dy / m]);
            let got = collide(&area, p, q);
            worst = worst.max((got[0] - g(a, 0)).abs()).max((got[1] - g(a, 2)).abs());
            n += 1;
        }
        assert!(n > 100, "{n} frames");
        assert!(worst < 1e-4, "worst {worst}");
    }
}
