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
    if let Some(p) = *PS2.lock().unwrap() {
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
