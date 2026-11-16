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
            out[k] = hermite(a.pos[k], b.pos[k], scale * a.param[k], scale * b.param[k], h, u, linear);
        }
        Some(out)
    }

    /// The parameter `T` the game's camera takes on this path for a subject at `subject` (`camera.md` §8.2,
    /// `SetCurrentSegment`): the subject is projected on the XZ line of each segment, the nearest segment wins (a tie: the
    /// smaller way along it), `T = (FrameNo[seg] + u ∓ C) / FrameNo[last]` with the camera `C = 30·scale` behind the
    /// projection (when the path follows: flag bits 1 and 3 clear). Not done: `GetCorrection` at the ends of paths of
    /// three keys and more (it matters only near a corner). `reverse` is the path's direction (`bReverse`).
    pub fn follow_t(&self, subject: [f32; 3], scale: f32, reverse: bool) -> Option<f32> {
        let last = self.keys.last()?.frame;
        if self.keys.len() < 2 || last <= 0.0 {
            return None;
        }
        let mut best: Option<(f32, f32, usize)> = None; // distance, way along, segment
        for i in 0..self.keys.len() - 1 {
            let (a, b) = (&self.keys[i], &self.keys[i + 1]);
            let (ax, az) = (b.pos[0] - a.pos[0], b.pos[2] - a.pos[2]);
            let (bx, bz) = (subject[0] - a.pos[0], subject[2] - a.pos[2]);
            let len = ax.hypot(az);
            if len == 0.0 {
                continue;
            }
            let u = (bx * ax + bz * az) / len;
            let d = ((az * bx - ax * bz) / len).abs();
            if best.is_none_or(|(bd, bu, _)| d < bd || (d == bd && u < bu)) {
                best = Some((d, u, i));
            }
        }
        let (_, u, i) = best?;
        let c = if self.flag & 0b1010 == 0 { scale * 30.0 } else { 0.0 };
        let t = (self.keys[i].frame + u + if reverse { c } else { -c }) / last;
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

type Mat = [[f32; 4]; 4];

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
}
