//! Pose of a `G101` skeleton at a frame of a `GIM0100` motion (У1.10): keys sampled per joint, then the
//! world matrices down the joint tree. Same arithmetic as the reference `gigpose.py` (checked on the runtime's
//! `CurrentKey` in X-08), in f64. Not done: the additive `StartBehavior` bit 2 and blending of motions.
use super::gig::Joint;
use super::gim::Motion;

pub type Mat = [[f64; 4]; 4];
pub type Key = [f64; 6];

const IDENTITY: Mat = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
const HALF_PI: f64 = 1.570_796_4;

pub fn mul(a: &Mat, b: &Mat) -> Mat {
    let mut m = [[0.0; 4]; 4];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

/// `T · Ry · Rz · Rx` of a key (position, rotation in radians).
pub fn key_matrix(k: &Key) -> Mat {
    let (cx, sx, cy, sy, cz, sz) = (k[3].cos(), k[3].sin(), k[4].cos(), k[4].sin(), k[5].cos(), k[5].sin());
    let rx = [[1.0, 0.0, 0.0, 0.0], [0.0, cx, -sx, 0.0], [0.0, sx, cx, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let ry = [[cy, 0.0, sy, 0.0], [0.0, 1.0, 0.0, 0.0], [-sy, 0.0, cy, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let rz = [[cz, -sz, 0.0, 0.0], [sz, cz, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let mut m = mul(&mul(&ry, &rz), &rx);
    m[0][3] = k[0];
    m[1][3] = k[1];
    m[2][3] = k[2];
    m
}

/// Parent of every joint from the first-child / next-sibling links (-1 for roots).
pub fn parents(joints: &[Joint]) -> Vec<i32> {
    let mut par = vec![-1; joints.len()];
    for (i, j) in joints.iter().enumerate() {
        let mut c = j.child as i32;
        let mut seen = std::collections::HashSet::new();
        while c >= 0 && (c as usize) < joints.len() && seen.insert(c) {
            par[c as usize] = i as i32;
            c = joints[c as usize].next as i32;
        }
    }
    par
}

/// World matrices from the keys, one per joint.
pub fn world(joints: &[Joint], keys: &[Key]) -> Vec<Mat> {
    let par = parents(joints);
    let mut w: Vec<Option<Mat>> = vec![None; joints.len()];
    fn get(i: usize, par: &[i32], keys: &[Key], w: &mut Vec<Option<Mat>>) -> Mat {
        if let Some(m) = w[i] {
            return m;
        }
        let m = key_matrix(&keys[i]);
        let m = if par[i] < 0 { m } else { mul(&get(par[i] as usize, par, keys, w), &m) };
        w[i] = Some(m);
        m
    }
    (0..joints.len()).map(|i| get(i, &par, keys, &mut w)).collect()
}

pub fn rest_keys(joints: &[Joint]) -> Vec<Key> {
    joints.iter().map(|j| j.init_key.map(f64::from)).collect()
}

/// Keys of every joint at `frame` (`GetKeyInterpolated`): the motion's joint is `STPTable[StandardJointNo]`
/// (0xFF: none, the rest key stays); linear between neighbouring keys, but for steps of up to 10 frames an axis
/// that jumps by more than π/2 takes the next key. A 0x20 key adds its position to the rest one, the rotation is
/// absolute; a rotation-only key keeps the rest position. `chara`: motion joint 0 is then absolute entirely.
pub fn sample(joints: &[Joint], m: &Motion, frame: f64, chara: bool) -> Vec<Key> {
    joints
        .iter()
        .map(|j| {
            let mut key = j.init_key.map(f64::from);
            let s = j.standard_joint;
            let k = if s >= 0 && (s as usize) < m.stp_table.len() { m.stp_table[s as usize] } else { 0xFF };
            let Some(gj) = (k != 0xFF).then(|| m.joints.get(k as usize)).flatten().and_then(|x| x.as_ref()) else { return key };
            if gj.keys.is_empty() {
                return key;
            }
            let ks = &gj.keys;
            let mut i = 0;
            while i + 1 < ks.len() && f64::from(ks[i + 1].frame) <= frame {
                i += 1;
            }
            let (k0, k1) = (&ks[i], &ks[(i + 1).min(ks.len() - 1)]);
            let (f0, f1) = (f64::from(k0.frame), f64::from(k1.frame));
            let t = if f1 == f0 || frame <= f0 { 0.0 } else { ((frame - f0) / (f1 - f0)).min(1.0) };
            let vals = |k: &super::gim::Key| -> Vec<f64> { k.pos.iter().flatten().chain(k.rot.iter()).map(|&x| f64::from(x)).collect() };
            let (v0, v1) = (vals(k0), vals(k1));
            let mut v: Vec<f64> = v0.iter().zip(&v1).map(|(x, y)| x + t * (y - x)).collect();
            if f1 - f0 <= 10.0 && t > 0.0 {
                for q in v.len() - 3..v.len() {
                    if (v1[q] - v0[q]).abs() > HALF_PI {
                        v[q] = v1[q];
                    }
                }
            }
            if gj.flag == 1 {
                key[3..6].copy_from_slice(&v);
            } else if k == 0 && chara {
                key.copy_from_slice(&v);
            } else {
                for a in 0..3 {
                    key[a] += v[a];
                }
                key[3..6].copy_from_slice(&v[3..6]);
            }
            key
        })
        .collect()
}

pub fn identity() -> Mat {
    IDENTITY
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joint(child: i16, next: i16, key: Key) -> Joint {
        Joint { is_bone_ref: false, key_no: 0, child, next, standard_joint: 0, init_key: key.map(|x| x as f32) }
    }

    #[test]
    fn chain_composes_translations_and_rotations() {
        // root rotated 90° about Y (x → -z), child 1 unit along x
        let js = [joint(1, -1, [0.0, 0.0, 0.0, 0.0, std::f64::consts::FRAC_PI_2, 0.0]), joint(-1, -1, [1.0, 0.0, 0.0, 0.0, 0.0, 0.0])];
        let w = world(&js, &rest_keys(&js));
        assert_eq!(parents(&js), vec![-1, 0]);
        assert!(w[1][0][3].abs() < 1e-6 && (w[1][2][3] + 1.0).abs() < 1e-6);
        assert_eq!(identity()[1][1], 1.0);
    }
}
