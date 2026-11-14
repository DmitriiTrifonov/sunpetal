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
