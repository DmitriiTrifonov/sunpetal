//! `sunpetal-ps2-mesh < RECORD > MESH`: the triangles of the PS2 models in an archive record, for the Python
//! catalogue tools (`tools/ps2/ps2mesh.py`). Format: `ps2::mesh::encode`.
use std::io::{Read, Write};

fn main() {
    let mut d = Vec::new();
    std::io::stdin().read_to_end(&mut d).expect("read stdin");
    let m = sunpetal_data::ps2::mesh::entry_mesh(&d);
    std::io::stdout().write_all(&sunpetal_data::ps2::mesh::encode(&m)).expect("write stdout");
}
