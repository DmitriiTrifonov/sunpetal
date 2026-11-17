//! Free camera, first hook (K-233, `docs/NATIVE-CAMERA.md`): after the game's `MTX_LookAt` (`0x02001E1C`, ARM) has
//! written the scene's view matrix (4x3, 1.19.12, at `0x02071108`) the host turns it by an extra yaw/pitch and moves
//! it. The game's own camera state is untouched; with the offsets at zero the matrix is left as it is.
//! On with `SUNPETAL_CAM=1` or the REPL's `cam` command.
use crate::abi::Cpu;
use crate::services::{mem_read32, mem_write32};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering::Relaxed};

const LOOKAT: u32 = 0x0200_1E1C;
const VIEW: u32 = 0x0207_1108;
/// the return address of the scene camera's call (`0x020260EA`): the other `LookAt` caller is left alone
const SCENE_CALL: std::ops::Range<u32> = 0x0202_60EA..0x0202_60F4;

/// The PS2 camera (K-235.3): place (PS2 units), pitch/yaw/roll (radians), the PS2 -> DS shift, transposed turn.
#[derive(Clone, Copy)]
struct Ps2 {
    eye: [f64; 3],
    angles: [f32; 3],
    shift: [f64; 3],
    transpose: bool,
}
static PS2: std::sync::Mutex<Option<Ps2>> = std::sync::Mutex::new(None);

/// The PS2 camera of a room that follows the hero (K-235.4; `camera.md` §8, §9): the camera area the hero is in gives the
/// path, `T` comes from the hero's place, the eye is the path at `T` (a change of path is blended over 360 frames), the
/// view looks at the hero (pitch at once, the yaw as the game's: a dead zone, `CamYaw`).
struct Follow {
    cam: sunpetal_data::ps2::export::Camera,
    areas: Vec<sunpetal_data::ps2::export::CamArea>,
    /// the area the hero is in now, the path in use and whether it is walked backwards
    area: Option<usize>,
    path: usize,
    reverse: bool,
    /// the yaw of the game's camera (its convention) and whether the path blends its turn (`bDelay`); the blend count and the eye it blends from
    yaw: Option<sunpetal_data::ps2::camera::CamYaw>,
    delay: bool,
    switch: u32,
    old_eye: [f32; 3],
    /// the eye of the last frame (export units)
    eye: [f32; 3],
    /// a path chosen by the user stays whatever the areas say
    fixed: bool,
}
static FOLLOW: std::sync::Mutex<Option<Follow>> = std::sync::Mutex::new(None);
/// The pointer to the world (its first word the hero, whose place is at +0x18, 1/4096) and the PS2 -> DS shift.
const WORLD: u32 = 0x0206_B45C + 4;
const SHIFT: [f64; 3] = [16.0, -18.5996, -13.0];

/// The camera's turn about y (radians, f32 bits), what the walk is relative to (`g_cameracorrection`).
static YAW_NOW: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The yaw the last PS2 view was built with (0 with the game's own camera).
pub fn yaw() -> f32 {
    f32::from_bits(YAW_NOW.load(Relaxed))
}

static ON: AtomicBool = AtomicBool::new(false);
static YAW: AtomicI32 = AtomicI32::new(0); // degrees * 100
static PITCH: AtomicI32 = AtomicI32::new(0);
static MOVE: [AtomicI32; 3] = [AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0)]; // 1.19.12 units, camera axes

pub fn wanted() -> bool {
    std::env::var("SUNPETAL_CAM").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// Sets the extra turn (degrees) and move (game units, camera axes) and switches the hook on.
pub fn set(yaw: f64, pitch: f64, mv: [f64; 3]) {
    YAW.store((yaw * 100.0) as i32, Relaxed);
    PITCH.store((pitch * 100.0) as i32, Relaxed);
    for i in 0..3 {
        MOVE[i].store((mv[i] * 4096.0) as i32, Relaxed);
    }
    install();
    ON.store(true, Relaxed);
}

/// Switches the PS2 camera on (`None`: off). With it on, the view after `MTX_LookAt` is built from the PS2 camera
/// instead of the game's: `shift` carries the PS2 scene into DS units, `transpose` flips the turn's convention.
pub fn set_ps2(eye: [f64; 3], pitch: f64, yaw: f64, roll: f64, shift: [f64; 3], transpose: bool) {
    *PS2.lock().unwrap() = Some(Ps2 { eye, angles: [pitch as f32, yaw as f32, roll as f32], shift, transpose });
    install();
    ON.store(true, Relaxed);
}

/// The PS2 camera from an exported path: `root` is `out/export/v1`, `record` the scene's camera record (`cameras/<scene>.json`
/// index), `path` the path in it, `t` its parameter. Returns the place and turn it took (PS2 units, radians).
pub fn set_ps2_path(root: &str, scene: &str, record: usize, path: usize, t: f32) -> Result<([f32; 3], [f32; 2]), String> {
    let ex = sunpetal_data::ps2::export::Export::open(root).map_err(|e| e.to_string())?;
    let cams = ex.cameras(scene).map_err(|e| e.to_string())?;
    let cam = cams.get(record).and_then(|c| c.camera.as_ref()).ok_or("no camera in that record")?;
    let p = cam.paths.get(path).ok_or("no such path")?;
    let pos = p.position(t, cam.scale).ok_or("path without keys")?;
    let rot = p.rotation(t).ok_or("path without keys")?;
    // the export's units are the scene's tenths (`scene_scale` 0,1): the DS world is the model's units
    set_ps2([pos[0] as f64 * 0.1, pos[1] as f64 * 0.1, pos[2] as f64 * 0.1], rot[0] as f64, rot[1] as f64, 0.0, [16.0, -18.5996, -13.0], false);
    Ok((pos, rot))
}

pub fn clear_ps2() {
    *PS2.lock().unwrap() = None;
    *FOLLOW.lock().unwrap() = None;
}

/// The camera runs along a path of the export after the hero (`cam ps2follow`): `T` from `CamPath::follow_t`, the turn
/// the path's own (the game's look at the subject, `camera.md` §5, is not done).
pub fn follow_ps2_path(root: &str, scene: &str, record: usize, path: Option<usize>) -> Result<(), String> {
    let ex = sunpetal_data::ps2::export::Export::open(root).map_err(|e| e.to_string())?;
    let cams = ex.cameras(scene).map_err(|e| e.to_string())?;
    let sc = cams.get(record).ok_or("no such record")?;
    let cam = sc.camera.clone().ok_or("no camera in that record")?;
    let first = path.unwrap_or(cam.start_path.max(0) as usize);
    if first >= cam.paths.len() {
        return Err("no such path".into());
    }
    *FOLLOW.lock().unwrap() = Some(Follow {
        cam,
        areas: sc.areas.clone(),
        area: None,
        path: first,
        reverse: false,
        yaw: None,
        delay: true,
        switch: 0,
        old_eye: [0.0; 3],
        eye: [0.0; 3],
        fixed: path.is_some(),
    });
    set_ps2([0.0; 3], 0.0, 0.0, 0.0, SHIFT, false);
    Ok(())
}

/// One frame of the follow: the area and path, `T`, the eye and the view for the hero at `s` (export units).
fn follow_frame(f: &mut Follow, p: &mut Ps2, s: [f32; 3]) {
    use std::f32::consts::PI;
    use sunpetal_data::ps2::camera::{select_area, CamYaw};
    if !f.fixed {
        let now = select_area(&f.areas, s, f.area);
        if let Some(i) = now {
            let a = &f.areas[i];
            if f.area != now && a.mode == 2 && (a.index as usize) < f.cam.paths.len() {
                if f.area.is_some() && f.path != a.index as usize {
                    f.switch = 360;
                    f.old_eye = f.eye;
                    // `SetCurrentPath` 0x126100: the turn blends unless the old and the new path both have bit 2 (`bDelayDisabled`)
                    f.delay = !(f.cam.paths[f.path].flag & 4 != 0 && f.cam.paths[a.index as usize].flag & 4 != 0);
                }
                f.path = a.index as usize;
                // `setPathDirection` keeps bit 0 of the area's `path_dir` (0x16a450): 1 (and -1) is backwards; no live frame with 1 was read back yet
                f.reverse = a.path_dir & 1 != 0;
            }
        }
        f.area = now;
    }
    let path = &f.cam.paths[f.path];
    let Some(t) = path.follow_t(s, f.cam.scale, f.reverse) else { return };
    let Some(pos) = path.position(t, f.cam.scale) else { return };
    let w = f.switch as f32 / 360.0;
    f.switch = f.switch.saturating_sub(1);
    let eye = [0, 1, 2].map(|i| pos[i] * (1.0 - w) + f.old_eye[i] * w);
    f.eye = eye;
    p.eye = eye.map(|v| v as f64 * 0.1);
    let (dx, dz) = (s[0] - eye[0], s[2] - eye[2]);
    let ty = s[1] + f.cam.scale * 15.0;
    let pitch = (ty - eye[1]).atan2(dx.hypot(dz)).max(-1.2566371);
    // the yaw (camera.md §9.2): the path's flag bit 1 set: its own turn blended 0.05/0.95; clear: a look at the hero with a dead
    // zone of π/N (N follows 24, or 10000 with bit 3; the game's 8 while running needs the hero's move state: not read); bit 4: a
    // continuous look
    let look = (-dx).atan2(dz);
    let cy = f.yaw.get_or_insert_with(|| CamYaw::new(look));
    if path.flag & 16 != 0 {
        cy.look_smooth(look);
    } else if path.flag & 2 != 0 {
        cy.follow_path(path.rotation(t).map_or(0.0, |r| r[1]), f.delay);
    } else {
        cy.look(look, if path.flag & 8 != 0 { 10000.0 } else { 24.0 });
    }
    let held = cy.yaw;
    // ours is the game's yaw turned: π - yaw (the same view)
    p.angles = [pitch, PI - held, 0.0];
}

/// The pitch and yaw (radians) that turn a PS2 camera at `eye` to `target` (the same units): the view's own convention,
/// which the follow reaches through the game's yaw (`π - yaw`); kept as the check of that conversion.
#[cfg(test)]
fn look_at(eye: [f64; 3], target: [f64; 3]) -> (f32, f32) {
    let d: Vec<f64> = (0..3).map(|i| target[i] - eye[i]).collect();
    ((d[1].atan2(d[0].hypot(d[2]))) as f32, (-d[0]).atan2(-d[2]) as f32)
}

type M = [i64; 12]; // columns of 3: x axis, y axis, z axis, translation

fn rot(yaw: f64, pitch: f64) -> [[f64; 3]; 3] {
    let (sy, cy) = yaw.to_radians().sin_cos();
    let (sp, cp) = pitch.to_radians().sin_cos();
    let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let rx = [[1.0, 0.0, 0.0], [0.0, cp, -sp], [0.0, sp, cp]];
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = (0..3).map(|k| rx[i][k] * ry[k][j]).sum();
        }
    }
    r
}

/// The DS view (4x3, 1.19.12, `x' = w0·x + w3·y + w6·z + w9`) of a PS2 camera. `glLookAt2`'s matrix works on row vectors
/// (`p' = p·A`), so `R[i][j] = A[j][i]`; the world is the DS one (`p_ds = p_ps2 + shift`, the same axes and units).
fn ps2_view(c: &Ps2) -> M {
    let a = sunpetal_data::ps2::camera::view_matrix([0.0; 3], c.angles[0], c.angles[1], c.angles[2] - std::f32::consts::PI);
    let r = |i: usize, j: usize| if c.transpose { a[i][j] } else { a[j][i] } as f64;
    let eye: Vec<f64> = (0..3).map(|i| c.eye[i] + c.shift[i]).collect();
    let mut out = [0i64; 12];
    for j in 0..3 {
        for i in 0..3 {
            out[3 * j + i] = (r(i, j) * 4096.0).round() as i64;
        }
    }
    for i in 0..3 {
        let t: f64 = -(0..3).map(|j| r(i, j) * eye[j]).sum::<f64>();
        out[9 + i] = (t * 4096.0).round() as i64;
    }
    out
}

/// `view' = (move, then turn about the camera) * view`: every column of the matrix is turned, the translation is moved.
pub fn adjust(m: &M, yaw: f64, pitch: f64, mv: [i64; 3]) -> M {
    let r = rot(yaw, pitch);
    let mut out = [0i64; 12];
    for col in 0..4 {
        for i in 0..3 {
            let v: f64 = (0..3).map(|k| r[i][k] * m[col * 3 + k] as f64).sum();
            out[col * 3 + i] = v.round() as i64;
        }
    }
    for i in 0..3 {
        out[9 + i] += mv[i];
    }
    out
}

unsafe extern "C" fn after(c: *mut Cpu) {
    let lr = (*c).pc & !1;
    if !ON.load(Relaxed) || !SCENE_CALL.contains(&lr) {
        return;
    }
    let mut ps2 = *PS2.lock().unwrap();
    if let (Some(p), Some(f)) = (ps2.as_mut(), FOLLOW.lock().unwrap().as_mut()) {
        let hero = mem_read32(c, WORLD);
        let hero = if hero != 0 { mem_read32(c, hero) } else { 0 };
        if hero != 0 {
            // the hero in the export's units: the DS world less the shift, tenths of it
            let at = |o: u32| (mem_read32(c, hero + o) as i32) as f64 / 4096.0;
            let s = [((at(0x18) - SHIFT[0]) * 10.0) as f32, ((at(0x1C) - SHIFT[1]) * 10.0) as f32, ((at(0x20) - SHIFT[2]) * 10.0) as f32];
            follow_frame(f, p, s);
        }
    }
    // the walk is relative to the game's `g_cameracorrection`: the yaw in the game's convention (ours is its π - yaw)
    YAW_NOW.store(ps2.map_or(0.0, |p| std::f32::consts::PI - p.angles[1]).to_bits(), Relaxed);
    if let Some(p) = ps2 {
        for (i, v) in ps2_view(&p).iter().enumerate() {
            mem_write32(c, VIEW + 4 * i as u32, *v as i32 as u32);
        }
        return;
    }
    let mut m: M = [0; 12];
    for (i, v) in m.iter_mut().enumerate() {
        *v = mem_read32(c, VIEW + 4 * i as u32) as i32 as i64;
    }
    let mv = [MOVE[0].load(Relaxed) as i64, MOVE[1].load(Relaxed) as i64, MOVE[2].load(Relaxed) as i64];
    let o = adjust(&m, YAW.load(Relaxed) as f64 / 100.0, PITCH.load(Relaxed) as f64 / 100.0, mv);
    for (i, v) in o.iter().enumerate() {
        mem_write32(c, VIEW + 4 * i as u32, *v as i32 as u32);
    }
}

pub fn install() {
    crate::machine(|m| m.after_fn(9, LOOKAT, after));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ps2_view_is_a_rigid_turn_at_the_eye() {
        let c = Ps2 { eye: [-17.4, 20.2, 9.0], angles: [0.1, -std::f32::consts::PI, 0.0], shift: [16.0, -18.6, -13.0], transpose: false };
        let m = ps2_view(&c);
        // the eye maps to the origin: R·eye + t = 0
        let e = [-1.4f64, 1.6, -4.0];
        for i in 0..3 {
            let v: f64 = (0..3).map(|j| m[3 * j + i] as f64 / 4096.0 * e[j]).sum::<f64>() + m[9 + i] as f64 / 4096.0;
            assert!(v.abs() < 0.01, "{v}");
        }
    }

    #[test]
    fn look_at_points_the_view_at_the_target() {
        let eye = [-17.4, 20.2, 9.0];
        let tgt = [-10.0, 18.0, 15.0];
        let (pitch, yaw) = look_at(eye, tgt);
        let m = ps2_view(&Ps2 { eye, angles: [pitch, yaw, 0.0], shift: [0.0; 3], transpose: false });
        let d: Vec<f64> = (0..3).map(|i| tgt[i] - eye[i]).collect();
        let n = d.iter().map(|v| v * v).sum::<f64>().sqrt();
        let dot: f64 = (0..3).map(|j| -(m[3 * j + 2] as f64) / 4096.0 * d[j] / n).sum();
        assert!(dot > 0.999, "{dot}");
    }

    #[test]
    fn zero_offsets_leave_the_matrix() {
        let m: M = [4096, 0, 0, 0, 4096, 0, 0, 0, 4096, 100, -200, 300];
        assert_eq!(adjust(&m, 0.0, 0.0, [0; 3]), m);
    }

    #[test]
    fn a_quarter_turn_carries_the_axes_and_the_translation() {
        let m: M = [4096, 0, 0, 0, 4096, 0, 0, 0, 4096, 100, 0, 0];
        let o = adjust(&m, 90.0, 0.0, [0, 0, 5]);
        // x axis -> (0,0,-1); translation (100,0,0) -> (0,0,-100) then +5
        assert_eq!(&o[0..3], &[0, 0, -4096]);
        assert_eq!(&o[9..12], &[0, 0, -95]);
    }
}
