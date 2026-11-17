//! The PS2 camera as the game computes it (K-235; `sunpetal-ee/notes/formats/camera.md` §3, §5, §6): the position and
//! turn of a path at the parameter `t` (`CCAMMANAGER::Spline`/`SplineRot`), the view matrix (`glLookAt2`) and the
//! screen distance of the projection. Pure functions of the exported data (`export::CamPath`).
use super::export::CamPath;

/// Hermite spline of one component between keys `i` and `i+1` (`nkeys < 3` is linear).
fn hermite(p0: f32, p1: f32, m0: f32, m1: f32, h: f32, u: f32, linear: bool) -> f32 {
    if h == 0.0 {
        return p0;
    }
    if linear {
        return p0 + u * (p1 - p0) / h;
    }
    p0 + u * (u * (3.0 * m0 + u * (m1 - m0) / h) + (p1 - p0) / h - h * (2.0 * m0 + m1))
}

impl CamPath {
    /// The segment `t` falls in (by `param_t`, clamped to the first and last).
    fn segment(&self, t: f32) -> Option<usize> {
        let n = self.keys.len();
        if n < 2 {
            return None;
        }
        // the last key whose param_t is not above t, but not the last one
        let i = self.keys.partition_point(|k| k.param_t <= t).saturating_sub(1);
        Some(i.min(n - 2))
    }

    /// The camera's place at `t` (`scale` is the header's `Scale`, the tangents' factor).
    pub fn position(&self, t: f32, scale: f32) -> Option<[f32; 3]> {
        if self.keys.len() == 1 {
            return Some(self.keys[0].pos);
        }
        let i = self.segment(t)?;
        let (a, b) = (&self.keys[i], &self.keys[i + 1]);
        let (h, u) = (b.param_t - a.param_t, t - a.param_t);
        let linear = self.keys.len() < 3;
        let mut out = [0.0; 3];
        for k in 0..3 {
            out[k] = {
                // the z tangent goes in with a minus (`camera.md` §9.2)
                let sign = if k == 2 { -1.0 } else { 1.0 };
                hermite(a.pos[k], b.pos[k], sign * scale * a.param[k], sign * scale * b.param[k], h, u, linear)
            };
        }
        Some(out)
    }

    /// The parameter `T` the game's camera takes on this path for a subject at `subject` (`camera.md` §9.1,
    /// `SetCurrentSegment` and `GetCorrection`): the subject is projected on the XZ line of each segment; the segment
    /// whose way along it is within the segment and whose distance is least wins (none within: least distance of all);
    /// the corner correction of the ends of a path of three keys and more; `T = (FrameNo[seg] + u - C - corr) / FrameNo[last]`
    /// with the camera `C = 30·scale` behind the projection. `reverse` is the path's direction (`bReverse`: `+C`).
    pub fn follow_t(&self, subject: [f32; 3], scale: f32, reverse: bool) -> Option<f32> {
        let last = self.keys.last()?.frame;
        if self.keys.len() < 2 || last <= 0.0 {
            return None;
        }
        // (inside the segment, distance, way along, segment, signed distance)
        let mut best: Option<(bool, f32, f32, usize, f32)> = None;
        for i in 0..self.keys.len() - 1 {
            let (a, b) = (&self.keys[i], &self.keys[i + 1]);
            let (ax, az) = (b.pos[0] - a.pos[0], b.pos[2] - a.pos[2]);
            let (bx, bz) = (subject[0] - a.pos[0], subject[2] - a.pos[2]);
            let len = ax.hypot(az);
            if len == 0.0 {
                continue;
            }
            let u = (bx * ax + bz * az) / len;
            let cross = az * bx - ax * bz;
            let d = if cross > 0.0 { -(cross.abs() / len) } else { cross.abs() / len };
            let inside = u >= 0.0 && u <= len;
            let better = match best {
                None => true,
                Some((bi, bd, bu, _, _)) => (inside && !bi) || (inside == bi && (d.abs() < bd || (d.abs() == bd && u < bu))),
            };
            if better {
                best = Some((inside, d.abs(), u, i, d));
            }
        }
        let (_, _, u, i, d) = best?;
        let (a, b) = (&self.keys[i], &self.keys[i + 1]);
        let len = (b.pos[0] - a.pos[0]).hypot(b.pos[2] - a.pos[2]);
        let h = len / 2.0;
        let mut corr = 0.0;
        if self.keys.len() >= 3 {
            let (theta, first) = if u < h { (a.theta, true) } else { (b.theta, false) };
            if theta.abs() > 0.01 {
                if first {
                    let r = d / theta.tan();
                    if r <= u {
                        corr = r * (h - u) / (h - r);
                    }
                } else {
                    let r = d / (-theta).tan();
                    if u <= len + r {
                        corr = r * (u - h) / (h + r);
                    }
                }
            }
            if corr.abs() > h || !corr.is_finite() {
                corr = 0.0;
            }
        }
        let c = scale * 30.0;
        let t = (a.frame + u + if reverse { c } else { -c } - corr) / last;
        Some(t.clamp(0.0, 1.0))
    }

    /// The camera's turn (about x, about y) at `t`: linear between the keys, a jump of more than a quarter turn is
    /// taken as a wrap of 2π (`SplineRot`).
    pub fn rotation(&self, t: f32) -> Option<[f32; 2]> {
        if self.keys.len() == 1 {
            return Some(self.keys[0].rot);
        }
        let i = self.segment(t)?;
        let (a, b) = (&self.keys[i], &self.keys[i + 1]);
        let (h, u) = (b.param_t - a.param_t, t - a.param_t);
        let mut out = [0.0; 2];
        for k in 0..2 {
            let mut d = b.rot[k] - a.rot[k];
            if d > std::f32::consts::FRAC_PI_2 {
                d -= std::f32::consts::TAU;
            } else if d < -std::f32::consts::FRAC_PI_2 {
                d += std::f32::consts::TAU;
            }
            out[k] = if h == 0.0 { a.rot[k] } else { a.rot[k] + u * d / h };
        }
        Some(out)
    }
}

/// Whether the area holds the point (x, y, z): its shape on the area's plane, the edge counted in (`ContainPoint`).
pub fn area_contains(a: &super::export::CamArea, p: [f32; 3]) -> bool {
    let (u, v) = match a.plane {
        1 => (p[2], p[0]),
        2 => (p[0], p[1]),
        3 => (p[1], p[2]),
        _ => return false,
    };
    let f = &a.fields;
    match (a.shape, f.len()) {
        (3, 4..) => u >= f[0] && u <= f[0] + f[2] && v >= f[1] && v <= f[1] + f[3],
        (2, 4..) => f[2] != 0.0 && f[3] != 0.0 && ((u - f[0]) / f[2]).powi(2) + ((v - f[1]) / f[3]).powi(2) <= 1.0,
        // a closed line: all the cross products of one sign (a zero is the edge, which counts in), `ContainPointClosedLine`
        (1, 6..) if f.len().is_multiple_of(2) => {
            let n = f.len() / 2;
            let (mut neg, mut pos) = (false, false);
            for i in 0..n {
                let (a, b) = ((f[2 * i], f[2 * i + 1]), (f[2 * ((i + 1) % n)], f[2 * ((i + 1) % n) + 1]));
                let c = (b.0 - a.0) * (v - a.1) - (b.1 - a.1) * (u - a.0);
                neg |= c < 0.0;
                pos |= c > 0.0;
            }
            !(neg && pos)
        }
        _ => false,
    }
}

/// The camera area the hero is in (`camera.md` §8.3): the current one while it holds the hero, else the first in the
/// table that does; `None` when no area does.
pub fn select_area(areas: &[super::export::CamArea], hero: [f32; 3], current: Option<usize>) -> Option<usize> {
    if let Some(c) = current.filter(|c| areas.get(*c).is_some_and(|a| area_contains(a, hero))) {
        return Some(c);
    }
    areas.iter().position(|a| area_contains(a, hero))
}

type Mat = [[f32; 4]; 4];

/// An angle taken into [-π, π].
fn wrap(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let mut a = a;
    if a > PI {
        a -= TAU;
    } else if a < -PI {
        a += TAU;
    }
    a
}

/// The yaw of the game's camera along a path (`SetCurrentPos` 0x126a40; `camera.md` §9.2, checked frame by frame by
/// `sunpetal-ee` Y-30/Y-32/Y-39): a look at the subject with a dead zone of `π/N` (the path's flag bit 1 clear), the
/// path's own turn blended 0.05/0.95 (bit 1 set), or a continuous look (`bPanDisabled`, bit 4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CamYaw {
    pub yaw: f32,
    /// the dead zone's width is `π/n`: `n` follows `n0` (24; 8 running; 10000 with the flag's bit 3) by 0.95/0.05 a frame
    pub n: f32,
    bit3: bool,
}

impl CamYaw {
    pub fn new(yaw: f32) -> CamYaw {
        CamYaw { yaw, n: 24.0, bit3: false }
    }

    /// One frame of the look at the subject: `want = atan2(eyeX - targetX, targetZ - eyeZ)`, `n0` the width's goal.
    pub fn look(&mut self, want: f32, n0: f32) {
        use std::f32::consts::PI;
        let old = self.n;
        self.n = 0.95 * self.n + 0.05 * n0;
        let (step_old, step_new) = (PI / old, PI / self.n);
        let diff = wrap(want - self.yaw);
        if diff.abs() > step_new {
            self.bit3 = true;
        }
        if self.bit3 {
            let s = if diff > 0.0 { 1.0 } else { -1.0 };
            let tmp = self.yaw + (diff - s * step_old);
            let before = self.yaw;
            self.yaw = if diff.abs() > step_new { self.yaw + (diff - s * step_new) } else { tmp };
            if (tmp - before).abs() < 0.01 {
                self.bit3 = false;
            }
            self.yaw = wrap(self.yaw);
        }
    }

    /// One frame of the path's own turn: the target (`SplineRot`) blended in by 0.05 when the path delays (`bDelay`),
    /// else taken at once; a target more than a half turn off is moved by 2π first.
    pub fn follow_path(&mut self, target: f32, delay: bool) {
        use std::f32::consts::TAU;
        let mut t = target;
        if t - self.yaw > std::f32::consts::PI {
            t -= TAU;
        } else if t - self.yaw < -std::f32::consts::PI {
            t += TAU;
        }
        self.yaw = wrap(if delay { 0.05 * t + 0.95 * self.yaw } else { t });
    }

    /// One frame of the continuous look (`PanMode` 2).
    pub fn look_smooth(&mut self, want: f32) {
        self.yaw = wrap(self.yaw + 0.05 * wrap(want - self.yaw));
    }
}

fn mul(a: &Mat, b: &Mat) -> Mat {
    let mut r = [[0.0; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            r[i][j] = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

fn rot_x(a: f32) -> Mat {
    let (s, c) = a.sin_cos();
    [[1.0, 0.0, 0.0, 0.0], [0.0, c, s, 0.0], [0.0, -s, c, 0.0], [0.0, 0.0, 0.0, 1.0]]
}

fn rot_y(a: f32) -> Mat {
    let (s, c) = a.sin_cos();
    [[c, 0.0, -s, 0.0], [0.0, 1.0, 0.0, 0.0], [s, 0.0, c, 0.0], [0.0, 0.0, 0.0, 1.0]]
}

fn rot_z(a: f32) -> Mat {
    let (s, c) = a.sin_cos();
    [[c, s, 0.0, 0.0], [-s, c, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]
}

/// The view matrix of `glLookAt2(eye, pitch, yaw, roll)`, row-major, the translation in row 3 (`g_vtm[12..14] = -eye`
/// before the turn is applied to it as the game does): `S · RotY(yaw) · RotX(pitch) · RotZ(-roll-π) · S`, `S = diag(1,1,-1)`.
pub fn view_matrix(eye: [f32; 3], pitch: f32, yaw: f32, roll: f32) -> Mat {
    let s: Mat = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, -1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let r = mul(&mul(&mul(&mul(&s, &rot_y(yaw)), &rot_x(pitch)), &rot_z(-roll - std::f32::consts::PI)), &s);
    let mut v = r;
    v[3] = [-eye[0], -eye[1], -eye[2], 1.0];
    v
}

/// The projection's distance to the screen: `cliph / (2·tan(fovy/2))` (the game's `Ang` is 60°, `cliph` 224).
pub fn scrdist(cliph: f32, fovy: f32) -> f32 {
    cliph / (2.0 * (fovy / 2.0).tan())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ps2::export::CamKey;

    fn key(t: f32, pos: [f32; 3], rot: [f32; 2], param: [f32; 3]) -> CamKey {
        CamKey { frame: 0.0, theta: 0.0, pos, rot, phai: 0.0, param_t: t, param }
    }

    fn path(keys: Vec<CamKey>) -> CamPath {
        CamPath { flag: 1, last_frame: 0, keys }
    }

    #[test]
    fn the_yaw_stands_in_its_dead_zone_and_follows_at_its_edge() {
        let mut y = CamYaw::new(0.0);
        // inside π/24: it stands
        for _ in 0..10 {
            y.look(0.1, 24.0);
        }
        assert_eq!(y.yaw, 0.0);
        // outside: it comes to within a step of the goal and stays there (R13: the rest is π/24, Y-30)
        for _ in 0..50 {
            y.look(1.0, 24.0);
        }
        let rest = (1.0 - y.yaw).abs();
        assert!((rest - std::f32::consts::PI / 24.0).abs() < 1e-3, "{}", rest);
        // the goal's n settles at n0
        assert!((y.n - 24.0).abs() < 1e-3);
    }

    #[test]
    fn the_path_turn_blends_and_flips_across_the_wrap() {
        // R01 room 1 path 4 (Y-39): -3.1355622 goes to the key's -3.1415925 by 0.05/0.95
        let mut y = CamYaw::new(-3.1355622);
        for _ in 0..119 {
            y.follow_path(-3.1415925, true);
        }
        assert!((y.yaw - -3.1415794).abs() < 2e-5, "{}", y.yaw);
        // path 5: yaw +π, target -π: the target is +π, the yaw stays
        let mut z = CamYaw::new(3.1355622);
        for _ in 0..119 {
            z.follow_path(-3.1415925, true);
        }
        assert!((z.yaw - 3.1415794).abs() < 2e-5, "{}", z.yaw);
        // no delay: at once
        z.follow_path(0.5, false);
        assert_eq!(z.yaw, 0.5);
    }

    #[test]
    fn two_keys_are_a_straight_line() {
        let p = path(vec![key(0.0, [0.0, 10.0, 0.0], [0.0, 0.0], [0.0; 3]), key(2.0, [4.0, 10.0, -8.0], [0.0, 1.0], [0.0; 3])]);
        assert_eq!(p.position(0.5, 1.0), Some([1.0, 10.0, -2.0]));
        assert_eq!(p.rotation(1.0), Some([0.0, 0.5]));
    }

    /// The cubic of `camera.md` §3 meets both keys of its segment whatever the tangent parameters are. (Against the
    /// runtime only the two-key line is checked so far: no frame with a path of three keys was read back.)
    #[test]
    fn three_keys_are_a_cubic_that_meets_the_keys() {
        let p = path(vec![
            key(0.0, [0.0, 1.0, 2.0], [0.0; 2], [0.3, -0.2, 0.7]),
            key(1.0, [5.0, 3.0, -1.0], [0.0; 2], [-0.4, 0.9, 0.1]),
            key(3.0, [2.0, -2.0, 4.0], [0.0; 2], [0.5, 0.5, -0.5]),
        ]);
        for (t, want) in [(0.0, [0.0, 1.0, 2.0]), (1.0, [5.0, 3.0, -1.0]), (3.0, [2.0, -2.0, 4.0])] {
            let q = p.position(t, 1.0).unwrap();
            assert!((0..3).all(|k| (q[k] - want[k]).abs() < 1e-4), "t {} got {:?} want {:?}", t, q, want);
        }
    }

    #[test]
    fn the_camera_follows_thirty_units_behind_the_projection() {
        let mut a = key(0.0, [0.0, 0.0, 0.0], [0.0; 2], [0.0; 3]);
        let mut b = key(100.0, [100.0, 0.0, 0.0], [0.0; 2], [0.0; 3]);
        (a.frame, b.frame) = (0.0, 100.0);
        let p = path(vec![a, b]);
        // subject at x 60 (3 off the line): u = 60, T = (60 - 30) / 100
        assert!((p.follow_t([60.0, 5.0, 3.0], 1.0, false).unwrap() - 0.3).abs() < 1e-6);
        assert!((p.follow_t([60.0, 5.0, 3.0], 1.0, true).unwrap() - 0.9).abs() < 1e-6);
        assert_eq!(p.follow_t([10.0, 0.0, 0.0], 1.0, false), Some(0.0));
    }

    #[test]
    fn an_area_holds_a_point_by_its_plane_and_keeps_the_current_one() {
        use super::super::export::CamArea;
        let rect = |plane, f: [f32; 4]| CamArea { index: 0, mode: 2, shape: 3, plane, path_dir: 0, fields: f.to_vec() };
        // plane 1 is (z, x): z 0..10, x 0..10
        let areas = vec![rect(1, [0.0, 0.0, 10.0, 10.0]), rect(1, [5.0, 5.0, 10.0, 10.0])];
        assert_eq!(select_area(&areas, [1.0, 0.0, 1.0], None), Some(0));
        // both hold (7, 7): the first in the table, but the current one stays
        assert_eq!(select_area(&areas, [7.0, 0.0, 7.0], None), Some(0));
        assert_eq!(select_area(&areas, [7.0, 0.0, 7.0], Some(1)), Some(1));
        assert_eq!(select_area(&areas, [50.0, 0.0, 50.0], Some(1)), None);
    }

    #[test]
    fn a_closed_line_holds_its_inside_edge_included_either_winding() {
        use super::super::export::CamArea;
        let tri = |f: [f32; 6]| CamArea { index: 0, mode: 2, shape: 1, plane: 2, path_dir: 0, fields: f.to_vec() };
        // plane 2 is (x, y)
        for a in [tri([0.0, 0.0, 10.0, 0.0, 0.0, 10.0]), tri([0.0, 0.0, 0.0, 10.0, 10.0, 0.0])] {
            assert!(area_contains(&a, [2.0, 2.0, 0.0]));
            assert!(area_contains(&a, [5.0, 5.0, 0.0]), "on the edge");
            assert!(!area_contains(&a, [8.0, 8.0, 0.0]));
            assert!(!area_contains(&a, [-1.0, 1.0, 0.0]));
        }
    }

    #[test]
    fn a_turn_across_the_wrap_takes_the_short_way() {
        let p = path(vec![key(0.0, [0.0; 3], [0.0, 3.0], [0.0; 3]), key(1.0, [0.0; 3], [0.0, -3.0], [0.0; 3])]);
        let y = p.rotation(0.5).unwrap()[1];
        // 3.0 → -3.0 is 0.283 forward across π, not 6 back
        assert!((y - (3.0 + (-3.0 + std::f32::consts::TAU - 3.0) / 2.0)).abs() < 1e-5, "{}", y);
    }

    #[test]
    fn the_screen_distance_is_the_games() {
        assert!((scrdist(224.0, std::f32::consts::FRAC_PI_3) - 193.98967).abs() < 1e-3);
    }

    #[test]
    fn the_view_turns_by_yaw_and_moves_by_minus_eye() {
        let v = view_matrix([1.0, 2.0, 3.0], 0.0, 0.0, -std::f32::consts::PI);
        assert_eq!(v[3], [-1.0, -2.0, -3.0, 1.0]);
        // no pitch/yaw, roll such that RotZ(0): the rotation is S·S = identity
        for i in 0..3 {
            for j in 0..3 {
                assert!((v[i][j] - if i == j { 1.0 } else { 0.0 }).abs() < 1e-6, "{} {} {}", i, j, v[i][j]);
            }
        }
    }

    /// R01, frame 6645 of the runtime (Y-13): path of two keys, `t` = 0.71593 → the game's `g_camposx/y/z`.
    #[test]
    fn the_real_export_gives_the_runtime_position() {
        let Some(root) = std::env::var_os("SUNPETAL_EE_EXPORT") else { return };
        let cams = crate::ps2::export::Export::open(root).unwrap().cameras("R01").unwrap();
        let found =
            cams.iter().filter_map(|c| c.camera.as_ref()).flat_map(|c| c.paths.iter().map(move |p| (c.scale, p))).filter(|(_, p)| p.keys.len() == 2 && p.flag == 1).find_map(
                |(s, p)| {
                    let q = p.position(0.71593, s)?;
                    ((q[0] + 181.1018).abs() < 1e-3 && (q[1] - 203.4204).abs() < 1e-3 && (q[2] - 90.0).abs() < 1e-3).then_some(q)
                },
            );
        assert!(found.is_some(), "no two-key path of R01 gives [-181.1018, 203.4204, 90]");
    }

    /// R13, record 0, path 2 (`out/y27/probe.json` of sunpetal-ee): the subject (the camera's `Target`) at six places; `T`
    /// within 1e-5 of the runtime's, the corner correction included.
    #[test]
    fn the_real_export_gives_the_runtime_t_with_the_corner_correction() {
        let Some(root) = std::env::var_os("SUNPETAL_EE_EXPORT") else { return };
        let root = std::path::PathBuf::from(root);
        let Ok(text) = std::fs::read_to_string(root.join("../../y27/probe.json")) else { return };
        let probe: serde_json::Value = serde_json::from_str(&text).unwrap();
        let cams = crate::ps2::export::Export::open(&root).unwrap().cameras("R13").unwrap();
        let cam = cams[0].camera.as_ref().unwrap();
        let path = &cam.paths[2];
        let n = probe.as_array().unwrap().len();
        for fr in probe.as_array().unwrap() {
            let g = |k: &str, i: usize| fr["gpa"][k][i].as_f64().unwrap() as f32;
            let want = fr["gpa"]["T"].as_f64().unwrap() as f32;
            let t = path.follow_t([g("target", 0), g("target", 1), g("target", 2)], cam.scale, false).unwrap();
            assert!((t - want).abs() < 1e-5, "frame {}: T {t} want {want}", fr["frame"]);
        }
        assert!(n >= 6, "{n} frames");
    }
}
