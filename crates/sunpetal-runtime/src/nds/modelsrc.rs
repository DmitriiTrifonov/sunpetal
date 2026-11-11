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

/// A model file seen in memory, kept between frames (`Nds::model_of`).
pub(super) struct File {
    base: u32,
    pub(super) size: u32,
    pub(super) model: String,
    /// The meshes' display lists: address and size.
    meshes: Vec<(u32, u32)>,
    /// The frame (`Gx::swaps`) the file was last checked to be still there.
    seen: u64,
    /// The replacement bound to this model (`gx.rs`, `Ult`), looked up once.
    pub(super) ult: Option<Option<usize>>,
}

/// The model files found so far and the stretches of memory found to hold none.
#[derive(Default)]
pub(super) struct Files {
    pub(super) list: Vec<File>,
    none: Vec<(u32, u32)>,
    cleared: u64,
    /// The first words of display lists that came by the CPU's copy, and where in a mesh they were found (0: nowhere).
    by_words: std::collections::HashMap<Vec<u32>, u32>,
}

impl Files {
    /// The address of a mesh's display list.
    pub(super) fn mesh_list(&self, file: usize, mesh: u16) -> u32 {
        self.list[file].meshes[mesh as usize].0
    }
}

impl super::Nds {
    /// The file (an index into `files.list`) and mesh a display-list address lies in, at frame `now`. Cheap
    /// after the first time: a file is checked once a frame, and memory with no file is remembered for 64 frames.
    pub(super) fn model_of(&mut self, files: &mut Files, addr: u32, now: u64) -> Option<(usize, u16)> {
        if addr == 0 {
            return None;
        }
        if now >= files.cleared + 64 {
            files.none.clear();
            files.by_words.retain(|_, a| *a != 0);
            files.cleared = now;
        }
        let mut i = 0;
        while i < files.list.len() {
            let f = &files.list[i];
            if addr >= f.base && addr - f.base < f.size {
                if f.seen != now {
                    let (base, size) = (f.base, f.size);
                    let same = self.m32(base) == BMD0 && self.m32(base + 8) == size && self.model_meshes(base).first().is_some_and(|m| m.0 == files.list[i].model);
                    if !same {
                        files.list.remove(i);
                        continue;
                    }
                    files.list[i].seen = now;
                }
                let f = &files.list[i];
                return f.meshes.iter().position(|m| addr >= m.0 && addr - m.0 < m.1).map(|m| (i, m as u16));
            }
            i += 1;
        }
        if files.none.iter().any(|n| addr >= n.0 && addr <= n.1) {
            return None;
        }
        let Some((base, size)) = self.model_file(addr) else {
            files.none.push((addr.saturating_sub(REACH).max(0x0200_0000), addr));
            return None;
        };
        let meshes = self.model_meshes(base);
        let model = meshes.first()?.0.clone();
        files.list.push(File { base, size, model, meshes: meshes.iter().map(|m| (m.3, m.4)).collect(), seen: now, ult: None });
        let f = files.list.last().unwrap();
        f.meshes.iter().position(|m| addr >= m.0 && addr - m.0 < m.1).map(|m| (files.list.len() - 1, m as u16))
    }
}

impl super::Nds {
    /// The file and mesh of a display list that did not come by DMA (a short list is copied to GXFIFO word by
    /// word, and its address is not seen): `words` are the ones that followed its BEGIN_VTXS, looked for in the
    /// meshes of the files seen so far. A model none of whose lists goes by DMA is not found.
    pub(super) fn model_by_words(&mut self, files: &mut Files, words: &[u32], now: u64) -> Option<(usize, u16)> {
        if words.len() < 3 {
            return None;
        }
        let at = match files.by_words.get(words) {
            Some(&a) => a,
            None => {
                let mut found = 0;
                let lists: Vec<(u32, u32)> = files.list.iter().flat_map(|f| f.meshes.iter().copied()).collect();
                'all: for (start, size) in lists {
                    let n = size / 4;
                    let list: Vec<u32> = (0..n).map(|i| self.m32(start + 4 * i)).collect();
                    if let Some(i) = list.windows(words.len()).position(|w| w == words) {
                        found = start + 4 * i as u32;
                        break 'all;
                    }
                }
                files.by_words.insert(words.to_vec(), found);
                found
            }
        };
        if at == 0 {
            return None;
        }
        let hit = self.model_of(files, at, now);
        if hit.is_none() {
            files.by_words.remove(words); // the file has gone
        }
        hit
    }
}
