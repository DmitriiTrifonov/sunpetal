//! Which model a batch of polygons comes from (`docs/ULTIMATE-IMPORT.md`, step У3.4). The game sends a
//! model's display lists to GXFIFO by DMA straight out of the model file in its memory, so the DMA's
//! source address names the file (`BMD0`), the model in it and the mesh whose display list holds the
//! address. A list goes out in pieces: the address is matched by range, not by a list's start.

/// A model's mesh, as the file in memory names them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelAt {
    /// The file's address.
    pub base: u32,
    /// The model's name in the file's dictionary (`no46_room_402r`).
    pub model: String,
    pub mesh: u16,
    pub mesh_name: String,
}

const BMD0: u32 = 0x3044_4D42;
const MDL0: u32 = 0x304C_444D;
/// How far below an address a file's start is looked for.
const REACH: u32 = 0x40_0000;

impl super::Nds {
    fn m32(&mut self, a: u32) -> u32 {
        self.rd(9, a, 4)
    }

    fn m16(&mut self, a: u32) -> u32 {
        self.rd(9, a, 2)
    }

    /// The entries of a resource dictionary at `d`: the name and the address of the entry's data.
    fn res_dict(&mut self, d: u32) -> Vec<(String, u32)> {
        let n = self.rd(9, d + 1, 1);
        let e = d + self.m16(d + 6);
        let (unit, names) = (self.m16(e), e + self.m16(e + 2));
        (0..n)
            .map(|i| {
                let name: Vec<u8> = (0..16).map(|k| self.rd(9, names + 16 * i + k, 1) as u8).take_while(|&c| c != 0).collect();
                (String::from_utf8_lossy(&name).into_owned(), e + 4 + i * unit)
            })
            .collect()
    }

    /// The model file that holds `addr`: its address and size, looked for downwards from `addr`.
    fn model_file(&mut self, addr: u32) -> Option<(u32, u32)> {
        if !(0x0200_0000..0x0240_0000).contains(&addr) {
            return None;
        }
        let mut a = addr & !3;
        let low = a.saturating_sub(REACH).max(0x0200_0000);
        while a >= low {
            if self.m32(a) == BMD0 && self.m16(a + 4) == 0xFEFF {
                let size = self.m32(a + 8);
                return (addr - a < size).then_some((a, size));
            }
            a -= 4;
        }
        None
    }

    /// The meshes of the file at `base`: model, mesh number, mesh name, the display list's address and size.
    fn model_meshes(&mut self, base: u32) -> Vec<(String, u16, String, u32, u32)> {
        let mut out = Vec::new();
        for k in 0..self.m16(base + 14).min(8) {
            let block = base + self.m32(base + 16 + 4 * k);
            if self.m32(block) != MDL0 {
                continue;
            }
            for (model, at) in self.res_dict(block + 8) {
                let mdl = block + self.m32(at);
                let shapes = mdl + self.m32(mdl + 12);
                for (i, (name, at)) in self.res_dict(shapes).into_iter().enumerate() {
                    let s = shapes + self.m32(at);
                    out.push((model.clone(), i as u16, name, s + self.m32(s + 8), self.m32(s + 12)));
                }
            }
        }
        out
    }

    /// The model and mesh behind each display-list address (`None`: not inside a model file's mesh).
    pub(crate) fn models_at(&mut self, addrs: &[u32]) -> Vec<Option<ModelAt>> {
        let mut files: Vec<(u32, u32, Vec<(String, u16, String, u32, u32)>)> = Vec::new();
        addrs
            .iter()
            .map(|&a| {
                let at = match files.iter().position(|f| a >= f.0 && a - f.0 < f.1) {
                    Some(i) => i,
                    None => {
                        let (base, size) = self.model_file(a)?;
                        let meshes = self.model_meshes(base);
                        files.push((base, size, meshes));
                        files.len() - 1
                    }
                };
                let (base, _, meshes) = &files[at];
                meshes.iter().find(|m| a >= m.3 && a - m.3 < m.4).map(|m| ModelAt { base: *base, model: m.0.clone(), mesh: m.1, mesh_name: m.2.clone() })
            })
            .collect()
    }
}
