//! The scene frame (K-228, `docs/NATIVE-RENDER-SEAM.md`): what the game gave the 3D engine in one frame, one level
//! above the polygons — camera and the models drawn (which model and mesh, which bone slot, where, how many
//! polygons). Read from the running machine; nothing is changed. The seam a free camera and the PS2 models
//! work at, instead of patches on the GX stream.
use crate::nds::debug;

pub const VIEW: u32 = 0x0207_1108; // 4x3, 1.19.12 (`nativecam.rs`)
pub const PROJ: u32 = 0x0207_10C4; // 4x4

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneItem {
    /// The DS model and mesh names; `None`: polygons whose source is not a known file (a batch not sent by DMA).
    pub model: Option<(String, String)>,
    pub mesh: u16,
    /// The matrix-stack slot of the last MTX_RESTORE before it: the bone of a skinned model (255: none).
    pub slot: u8,
    pub polys: u32,
    /// The position matrix (view * model) at its first batch, 4x4, 1.19.12.
    pub pos: [i32; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneFrame {
    pub view: [i32; 12],
    pub proj: [i32; 16],
    pub items: Vec<SceneItem>,
}

fn words<const N: usize>(addr: u32) -> [i32; N] {
    let b = debug::peek(addr, N * 4).unwrap_or_else(|| vec![0; N * 4]);
    std::array::from_fn(|i| i32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]))
}

impl SceneFrame {
    /// The frame the 3D engine draws now.
    pub fn capture() -> SceneFrame {
        let items = debug::frame_models()
            .into_iter()
            .map(|(m, polys, slot, pos)| SceneItem { mesh: m.as_ref().map_or(0, |m| m.mesh), model: m.map(|m| (m.model, m.mesh_name)), slot, polys, pos })
            .collect();
        SceneFrame { view: words(VIEW), proj: words(PROJ), items }
    }

    /// FNV-1a over everything in the frame: equal frames, equal hashes.
    pub fn hash(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut eat = |b: &[u8]| {
            for x in b {
                h = (h ^ *x as u64).wrapping_mul(0x100_0000_01b3);
            }
        };
        for v in self.view.iter().chain(self.proj.iter()) {
            eat(&v.to_le_bytes());
        }
        for it in &self.items {
            if let Some((a, b)) = &it.model {
                eat(a.as_bytes());
                eat(b"\0");
                eat(b.as_bytes());
            }
            eat(&[0, it.slot]);
            eat(&it.mesh.to_le_bytes());
            eat(&it.polys.to_le_bytes());
            for v in &it.pos {
                eat(&v.to_le_bytes());
            }
        }
        h
    }

    /// Polygons that belong to a known model.
    pub fn named_polys(&self) -> u32 {
        self.items.iter().filter(|i| i.model.is_some()).map(|i| i.polys).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_follows_every_field() {
        let item = SceneItem { model: Some(("a".into(), "b".into())), mesh: 1, slot: 2, polys: 3, pos: [0; 16] };
        let f = SceneFrame { view: [0; 12], proj: [0; 16], items: vec![item.clone()] };
        let mut g = f.clone();
        assert_eq!(f.hash(), g.hash());
        g.items[0].pos[15] = 4096;
        assert_ne!(f.hash(), g.hash());
        g = f.clone();
        g.view[0] = 1;
        assert_ne!(f.hash(), g.hash());
        assert_eq!(f.named_polys(), 3);
    }
}
