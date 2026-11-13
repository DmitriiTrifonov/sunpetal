//! Triangles of the models in an archive record, in the rest pose (У1.13): what the catalogue of PS2 models
//! (`tools/ps2/catalog.py`) draws. Every `G101` model found in the bytes is read with [`super::gig`], its
//! vertices are moved by the world matrix of their bone ([`super::pose`] on the `InitKey` pose) and strips and
//! lists become triangles, wound the way the faces look out (F-12).
use super::{gig, pose};

/// A vertex as the catalogue's tools read it: x y z 1, nx ny nz 1, r g b 128, u v 1 0.
pub type Vertex = [f32; 16];

#[derive(Debug, Default)]
pub struct Mesh {
    pub triangles: Vec<[Vertex; 3]>,
    /// Series that are triangle lists / strips.
    pub lists: u32,
    pub strips: u32,
    /// Triangles dropped as degenerate (a repeated or non-finite point).
    pub degenerate: u32,
}

fn apply(m: &pose::Mat, v: [f32; 3], translate: bool) -> [f32; 3] {
    let p = [f64::from(v[0]), f64::from(v[1]), f64::from(v[2])];
    let t = if translate { 1.0 } else { 0.0 };
    std::array::from_fn(|i| (m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2] + t * m[i][3]) as f32)
}

fn vertex(s: &gig::Series, v: &gig::Vertex, world: &[pose::Mat]) -> Vertex {
    let bone = s.bones.get(v.slot as usize).or(s.bones.first()).map_or(0, |&b| b as usize);
    let (pos, nrm) = match world.get(bone).or(world.first()) {
        Some(m) => (apply(m, v.pos, true), apply(m, v.nrm, false)),
        None => (v.pos, v.nrm),
    };
    [pos[0], pos[1], pos[2], 1.0, nrm[0], nrm[1], nrm[2], 1.0, v.rgba[0], v.rgba[1], v.rgba[2], 128.0, v.st[0], v.st[1], 1.0, 0.0]
}

/// All `G101` models of `d` (a record of an archive) as one mesh.
pub fn entry_mesh(d: &[u8]) -> Mesh {
    let mut mesh = Mesh::default();
    for at in gig::find(d) {
        let Ok(g) = gig::parse(&d[at..]) else { continue };
        for o in &g.objects {
            let world = pose::world(&o.joints, &pose::rest_keys(&o.joints));
            for s in &o.series {
                let vs: Vec<Vertex> = s.verts.iter().map(|v| vertex(s, v, &world)).collect();
                let tris: Vec<[usize; 3]> = match s.kind() {
                    4 => (0..vs.len().saturating_sub(2)).map(|i| if i % 2 == 0 { [i, i + 2, i + 1] } else { [i + 1, i + 2, i] }).collect(),
                    3 => (0..vs.len() / 3).map(|i| [3 * i, 3 * i + 2, 3 * i + 1]).collect(),
                    _ => continue,
                };
                if s.kind() == 4 {
                    mesh.strips += 1;
                } else {
                    mesh.lists += 1;
                }
                for [a, b, c] in tris {
                    let t = [vs[a], vs[b], vs[c]];
                    let finite = t.iter().all(|v| v[..3].iter().all(|x| x.is_finite()));
                    let distinct = t[0][..3] != t[1][..3] && t[1][..3] != t[2][..3] && t[0][..3] != t[2][..3];
                    if finite && distinct {
                        mesh.triangles.push(t);
                    } else {
                        mesh.degenerate += 1;
                    }
                }
            }
        }
    }
    mesh
}

/// The byte form the Python tools read: `u32 lists, strips, degenerate, triangles`, then 3 vertices of 16 `f32` per triangle.
pub fn encode(m: &Mesh) -> Vec<u8> {
    let mut o = Vec::with_capacity(16 + m.triangles.len() * 192);
    for n in [m.lists, m.strips, m.degenerate, m.triangles.len() as u32] {
        o.extend_from_slice(&n.to_le_bytes());
    }
    for v in m.triangles.iter().flatten().flatten() {
        o.extend_from_slice(&v.to_le_bytes());
    }
    o
}
