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
    for e in &mut area.events {
        e.enabled = false;
        e.refer_enabled = false;
    }
    let mut rows = zones_tsv.lines();
    let Some(head) = rows.next() else { return };
    let col = |n: &str| head.split('\t').position(|h| h == n);
    let (Some(cs), Some(c3), Some(ck), Some(cz), Some(ce)) = (col("scene"), col("scene3d"), col("kind"), col("zone"), col("initial_enabled")) else { return };
    let cb = col("initial_bits");
    for row in rows {
        let c: Vec<&str> = row.split('\t').collect();
        if c.len() <= ce || c[cs] != scene || c[c3].parse() != Ok(scene3d) {
            continue;
        }
        let (Ok(z), on) = (c[cz].parse::<usize>(), c[ce] == "1") else { continue };
        match c[ck] {
            "hit" => area.hit.get_mut(z).iter_mut().for_each(|h| h.enabled = on),
            "height" => area.height.get_mut(z).iter_mut().for_each(|t| t.enabled = on),
            // the save byte of an event zone: bit 0 on, bit 1 its reference point on
            "event" => {
                let bits = cb.and_then(|b| c.get(b)).and_then(|v| v.parse::<u32>().ok()).unwrap_or(on as u32);
                area.events.get_mut(z).iter_mut().for_each(|e| (e.enabled, e.refer_enabled) = (bits & 1 != 0, bits & 2 != 0));
            }
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

/// The fast run's speed for Seleka: `FastRunSpeed` 14 times `GIG+0x20` 0.1 (`walk.md` §3).
pub const SELEKA_FAST: f32 = 1.4;

/// Whether `AutoFastRun` leaves the fast run allowed at the hero `hero` (x, y, z): every event zone that is on, with its reference point on, an
/// event and a static condition bit 0 clear forbids it within 150 units (the plane x-z; y too when condition bit 3 is set), `walk.md` §3a.
/// Zones whose point follows a character or a model (`refer_type` 1, 2: a joint) are not read here and do not forbid.
pub fn fast_run_allowed(area: &Area3d, hero: [f32; 3]) -> bool {
    !area.events.iter().any(|e| {
        let y = if e.condition & 8 != 0 { (e.refer[1] - hero[1]).powi(2) } else { 0.0 };
        e.enabled
            && e.refer_enabled
            && e.refer_type == 0
            && e.event_id >= 0
            && e.condition & 1 == 0
            && (e.refer[0] - hero[0]).powi(2) + (e.refer[2] - hero[2]).powi(2) + y < 22500.0
    })
}

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

/// The direction the hero faces for a stick (0 is +z, π/2 is +x), `SetTargetDirection` (`walk.md` §4, Y-28): the stick's own angle,
/// without the camera (the camera's `−π−corr` is the angle of the step, not of the face).
pub fn stick_direction(stick: (i8, i8)) -> Option<f32> {
    (stick != (0, 0)).then(|| (stick.0 as f32).atan2(stick.1 as f32))
}

/// The face and the move state of the hero (`walk.md` §4): the move of a frame goes by the state the previous frame left
/// (0 stands: the first frame of a walk, and the frame a big turn starts, never moves), a turn in progress holds the walk.
#[derive(Clone, Copy, Debug, Default)]
pub struct Walker {
    pub face: f32,
    pub turn: Option<Turn>,
    /// 0 stands, 1 walks (a turn goes on), 2 free to run, 3 the fast run (`MoveState` of the game)
    pub state: u8,
}

impl Walker {
    /// One frame for the stick; returns the state the step of this frame goes by (0 no step, 1 walk speed, 2 the stick's own).
    pub fn frame(&mut self, stick: (i8, i8)) -> u8 {
        self.frame_fast(stick, false)
    }

    /// The same, with the fast run `fast` (allowed and on: `isFastRunMode`, `walk.md` §3a): free of a turn and the stick past 120, the state is 3.
    pub fn frame_fast(&mut self, stick: (i8, i8), fast: bool) -> u8 {
        let used = self.state;
        match stick_direction(stick) {
            None => {
                self.state = 0;
                self.turn = None;
            }
            Some(target) => {
                self.face = face_step(&mut self.turn, self.face, target);
                let running = self.turn.is_some_and(|t| t.done < t.total);
                self.state = if running {
                    1
                } else if fast && (stick.0 as f32).hypot(stick.1 as f32) > 120.0 {
                    3
                } else {
                    2
                };
            }
        }
        used
    }
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

/// Whether the point (x, z) is in the shape (the border is in), as `ContainPoint` for an event zone.
pub fn shape_contains(shape: &HitShape, p: [f32; 2]) -> bool {
    match shape {
        HitShape::Rect { x0, z0, dx, dz } => p[0] >= *x0 && p[0] <= x0 + dx && p[1] >= *z0 && p[1] <= z0 + dz,
        HitShape::Ellipse { cx, cz, rx, rz } => ((p[0] - cx) / rx).powi(2) + ((p[1] - cz) / rz).powi(2) <= 1.0,
        HitShape::Polygon(poly) => poly.len() >= 3 && contain_closed(poly, p) >= 0,
    }
}

/// The automatic event zones the hero stands in (`event-conditions.md` §1–2): on (save bit 0), `Condition` bit 0 (no button),
/// with an event; bit 2 checks the raw place, else the point 2 ahead along the face (0 is +z, π/2 is +x). (zone, event id).
pub fn auto_events(area: &Area3d, raw: [f32; 2], face: f32) -> Vec<(usize, i32)> {
    let fwd = [raw[0] + 2.0 * face.sin(), raw[1] + 2.0 * face.cos()];
    area.events
        .iter()
        .enumerate()
        .filter(|(_, e)| e.enabled && e.condition & 1 != 0 && e.event_id >= 0)
        .filter(|(_, e)| e.shape.as_ref().is_some_and(|s| shape_contains(s, if e.condition & 4 != 0 { raw } else { fwd })))
        .map(|(i, e)| (i, e.event_id))
        .collect()
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

/// Whether switching one zone can change where the hero walks (`kind` "hit" or "height"): a wall matters if some step
/// on the floor ends differently with it on and off (the other walls as they are, or all on); a floor triangle matters
/// if it covers a point no other triangle does. Sampled on a grid of at most ~40k points per zone, so a sliver thinner
/// than the step can be missed. Used to tell the zone events that matter from decoration (`docs/NEXT.md`, Y-50).
pub fn zone_matters(area: &Area3d, kind: &str, zone: usize) -> bool {
    let mut all = area.clone();
    all.height.iter_mut().for_each(|t| t.enabled = true);
    let bbox =
        |pts: &mut dyn Iterator<Item = [f32; 2]>| pts.fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
    let grid = |b: [f32; 4], pad: f32| {
        let (x0, z0, x1, z1) = (b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad);
        let step = 2.0f32.max(((x1 - x0) * (z1 - z0) / 40000.0).sqrt());
        let (nx, nz) = (((x1 - x0) / step) as i32 + 1, ((z1 - z0) / step) as i32 + 1);
        (0..nx).flat_map(move |i| (0..nz).map(move |j| [x0 + i as f32 * step, z0 + j as f32 * step])).collect::<Vec<_>>()
    };
    match kind {
        "height" => {
            let Some(t) = area.height.get(zone) else { return false };
            if t.points.len() != 3 {
                return false;
            }
            let b = bbox(&mut t.points.iter().map(|p| [p[0], p[2]]));
            let mut off = all.clone();
            off.height[zone].enabled = false;
            grid(b, 0.0).into_iter().any(|p| floor_height(&all, p[0], p[1]).is_some() && floor_height(&off, p[0], p[1]).is_none())
        }
        "hit" => {
            let Some(h) = area.hit.get(zone) else { return false };
            let b = match &h.shape {
                HitShape::Rect { x0, z0, dx, dz } => [*x0, *z0, x0 + dx, z0 + dz],
                HitShape::Ellipse { cx, cz, rx, rz } => [cx - rx, cz - rz, cx + rx, cz + rz],
                HitShape::Polygon(pts) => bbox(&mut pts.iter().copied()),
            };
            let pts = grid(b, 4.0);
            let step = 2.0;
            let dirs = [[1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0], [0.7, 0.7], [-0.7, 0.7], [0.7, -0.7], [-0.7, -0.7]];
            // the bases: the walls as the room starts with, and all of them on
            let mut every = all.clone();
            every.hit.iter_mut().for_each(|h| h.enabled = true);
            [&all, &every].iter().any(|base| {
                let (mut on, mut off) = ((*base).clone(), (*base).clone());
                on.hit[zone].enabled = true;
                off.hit[zone].enabled = false;
                pts.iter().filter(|p| floor_height(&all, p[0], p[1]).is_some()).any(|p| {
                    dirs.iter().any(|d| {
                        let q = [p[0] + d[0] * step, p[1] + d[1] * step];
                        collide(&on, *p, q) != collide(&off, *p, q)
                    })
                })
            })
        }
        _ => false,
    }
}

/// One frame of the hero's walk on the ground: `pos` (x, y, z) goes by the stick and the walls, y to the floor.
pub fn walk_frame(area: &Area3d, pos: [f32; 3], stick: (i8, i8), camera_correction: f32, s: Speeds) -> [f32; 3] {
    let st = stick_step(stick, camera_correction, s);
    let q = collide(area, [pos[0], pos[2]], [pos[0] + st[0], pos[2] + st[1]]);
    [q[0], floor_height(area, q[0], q[1]).unwrap_or(pos[1]), q[1]]
}

#[cfg(test)]
mod tests {
    use super::super::export::{EventZone, HeightTri, Hit};
    use super::*;

    fn area(hit: Vec<Hit>) -> Area3d {
        Area3d { index: 0, record: 0, hit, height: vec![], events: vec![] }
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

    /// The face and the state of the live walk of R01 (`out/y21/probe.json`, three runs): the face of each frame within 1e-3, the state exactly.
    #[test]
    fn the_walker_follows_the_live_face_and_state() {
        let Some(root) = std::env::var_os("SUNPETAL_EE_EXPORT") else { return };
        let Ok(text) = std::fs::read_to_string(std::path::PathBuf::from(root).join("../../y21/probe.json")) else { return };
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        for run in ["straight", "diagonal", "wall"] {
            let fr = v[run].as_array().unwrap();
            let mut w = Walker { face: fr[0]["rot"].as_f64().unwrap() as f32, ..Default::default() };
            for r in fr {
                let b = |k: &str| r[k].as_i64().unwrap() as u8 as i8;
                w.frame((b("dx"), b("dy")));
                assert_eq!(w.state as i64, r["state"].as_i64().unwrap(), "{run} frame {}", r["frame"]);
                // a rest frame's face is the game's own (a script turned it)
                if w.state == 0 {
                    w.face = r["rot"].as_f64().unwrap() as f32;
                    continue;
                }
                let want = r["rot"].as_f64().unwrap() as f32;
                assert!(wrap(w.face - want).abs() < 1e-3, "{run} frame {}: face {} want {}", r["frame"], w.face, want);
            }
        }
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

    #[test]
    fn a_wall_matters_only_where_the_hero_can_walk() {
        let tri = |x: f32| HeightTri { enabled: true, points: vec![[x, 0.0, 0.0, 1.0], [x + 100.0, 0.0, 0.0, 1.0], [x, 0.0, 100.0, 1.0]] };
        let mut a = area(vec![rect(10.0, 10.0, 5.0, 5.0), rect(500.0, 500.0, 5.0, 5.0)]);
        a.height = vec![tri(0.0)];
        assert!(zone_matters(&a, "hit", 0));
        assert!(!zone_matters(&a, "hit", 1));
        a.height.push(tri(0.0));
        assert!(!zone_matters(&a, "height", 0), "a floor under another floor");
        a.height.push(tri(300.0));
        assert!(zone_matters(&a, "height", 2));
    }

    /// With the real export (`SUNPETAL_EE_EXPORT`): which zones with on/off events matter for walking, to `build/zones/relevance.tsv`.
    #[test]
    fn the_real_zones_with_events_are_sorted_by_whether_they_matter() {
        let Some(root) = std::env::var_os("SUNPETAL_EE_EXPORT") else { return };
        let root = std::path::PathBuf::from(root);
        let zones = std::fs::read_to_string(root.join("zones.tsv")).unwrap();
        let e = super::super::export::Export::open(&root).unwrap();
        let h: Vec<&str> = zones.lines().next().unwrap().split('\t').collect();
        let ix = |n: &str| h.iter().position(|x| *x == n).unwrap();
        let (cs, c3, ck, cz, con, coff) = (ix("scene"), ix("scene3d"), ix("kind"), ix("zone"), ix("on_events"), ix("off_events"));
        let mut cache: std::collections::HashMap<(String, usize), Area3d> = Default::default();
        let mut out = String::from("scene\tscene3d\tkind\tzone\tmatters\n");
        let (mut n, mut m) = (0, 0);
        for row in zones.lines().skip(1) {
            let c: Vec<&str> = row.split('\t').collect();
            if c.len() <= coff || !matches!(c[ck], "hit" | "height") || (c[con].is_empty() && c[coff].is_empty()) {
                continue;
            }
            let (scene, s3, z) = (c[cs].to_string(), c[c3].parse::<usize>().unwrap(), c[cz].parse::<usize>().unwrap());
            let area = cache.entry((scene.clone(), s3)).or_insert_with(|| {
                let mut a = e.scene(&scene).unwrap().areas.into_iter().find(|a| a.index as usize == s3).unwrap();
                apply_initial_zones(&mut a, &zones, &scene, s3);
                a
            });
            let yes = zone_matters(area, c[ck], z);
            n += 1;
            m += yes as usize;
            out += &format!("{}\t{}\t{}\t{}\t{}\n", scene, s3, c[ck], z, yes as u8);
        }
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../build/zones");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("relevance.tsv"), out).unwrap();
        eprintln!("zones with events: {}, matter for walking: {}", n, m);
    }

    #[test]
    fn an_event_point_forbids_the_fast_run_within_150_units() {
        // `y22/fastrun_check.tsv`: a point 149 away forbids, 151 and 200 allow, none is allowed
        let zone = |d: f32| EventZone { enabled: true, refer_enabled: true, refer: [d, 99.0, 0.0], refer_type: 0, condition: 0, event_id: 3, shape: None };
        let mut a = area(vec![]);
        assert!(fast_run_allowed(&a, [0.0; 3]));
        a.events = vec![zone(149.0)];
        assert!(!fast_run_allowed(&a, [0.0; 3]), "149 forbids; y does not count");
        a.events = vec![zone(151.0)];
        assert!(fast_run_allowed(&a, [0.0; 3]));
        a.events = vec![EventZone { enabled: false, ..zone(10.0) }];
        assert!(fast_run_allowed(&a, [0.0; 3]), "a zone that is off does not forbid");
        a.events = vec![EventZone { condition: 8, ..zone(10.0) }];
        assert!(fast_run_allowed(&a, [0.0, 0.0, 0.0]) == ((99.0f32).powi(2) + 100.0 >= 22500.0), "y counts with condition bit 3");
    }

    #[test]
    fn an_automatic_event_zone_fires_by_the_raw_place_or_the_point_ahead() {
        // R11 room 5 (the lighthouse), event zones 3 (#39) and 5 (#40): rectangles side by side, condition 5 (auto, raw place)
        let rect = |x0: f32, z0: f32, dx: f32, dz: f32, ev: i32, cond: u32| EventZone {
            enabled: true,
            refer_enabled: false,
            refer: [0.0; 3],
            refer_type: 0,
            condition: cond,
            event_id: ev,
            shape: Some(HitShape::Rect { x0, z0, dx, dz }),
        };
        let mut a = area(vec![]);
        a.events = vec![rect(-1848.42, 174.566, 20.844, 8.848, 39, 5), rect(-1845.033, 183.218, 10.238, 6.98, 40, 5)];
        assert_eq!(auto_events(&a, [-1840.0, 178.0], 0.0), vec![(0, 39)]);
        assert_eq!(auto_events(&a, [-1840.0, 188.0], 0.0), vec![(1, 40)]);
        assert!(auto_events(&a, [-1800.0, 178.0], 0.0).is_empty());
        a.events[0].enabled = false;
        assert!(auto_events(&a, [-1840.0, 178.0], 0.0).is_empty(), "a zone that is off does not fire");
        // condition 1: the point 2 ahead; a button zone (bit 0 clear) never fires by itself
        a.events = vec![rect(0.0, 10.0, 5.0, 5.0, 7, 1)];
        assert!(auto_events(&a, [2.0, 8.5], std::f32::consts::PI).is_empty());
        assert_eq!(auto_events(&a, [2.0, 8.5], 0.0), vec![(0, 7)]);
        a.events[0].condition = 4;
        assert!(auto_events(&a, [2.0, 12.0], 0.0).is_empty());
    }

    #[test]
    fn the_fast_run_comes_after_the_turn_and_only_with_the_stick_pushed_far() {
        // `y22`: a turn (state 1) first, then state 3 while the stick is at the end; a half-pushed stick runs plain
        let mut w = Walker::default();
        let mut seen = vec![];
        for _ in 0..40 {
            seen.push(w.frame_fast((127, 0), true));
        }
        assert_eq!(seen[0], 0);
        assert!(seen.contains(&1) && *seen.last().unwrap() == 3);
        assert!(seen.iter().skip_while(|s| **s != 3).all(|s| *s == 3), "once fast, it stays");
        let mut w = Walker::default();
        let plain: Vec<u8> = (0..40).map(|_| w.frame_fast((100, 0), true)).collect();
        assert!(!plain.contains(&3));
        let mut w = Walker::default();
        assert!((0..40).all(|_| w.frame_fast((127, 0), false) != 3), "not allowed: never");
    }
}
