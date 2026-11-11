"""sunpetal-ee export (models/<id>.json + .vtx.bin) -> SPM2 (crates/sunpetal-data/src/spm2.rs).
usage: ee2spm.py MODELS_DIR ID OUT.spm [--bind DS_NAME [--size N] [--anchor MESH] [--place 12 floats]]
Without --bind the model is unbound (flags 0). Pictures are not written yet (step У2.4): every material has none.
Several objects are merged: their bones follow one another, a vertex's bone is the series' bone for its slot."""
import argparse
import json
import os
import struct

ap = argparse.ArgumentParser()
ap.add_argument("dir")
ap.add_argument("id")
ap.add_argument("out")
ap.add_argument("--bind")
ap.add_argument("--size", type=int, default=0)
ap.add_argument("--anchor", type=lambda s: int(s, 0), default=0xFFFF)
ap.add_argument("--place", type=float, nargs=12, default=[1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0])
a = ap.parse_args()

j = json.load(open(os.path.join(a.dir, a.id + ".json")))
vtx = open(os.path.join(a.dir, a.id + ".vtx.bin"), "rb").read()


def tris(kind, n):
    if kind == "strip":
        return [(i, i + 2, i + 1) if i % 2 == 0 else (i + 1, i + 2, i) for i in range(n - 2)]
    if kind == "list":
        return [(i, i + 2, i + 1) for i in range(0, n - 2, 3)]
    return []


# bones of all objects, one after another
bones, base = [], []
for ob in j["objects"]:
    off = len(bones)
    base.append(off)
    for q in ob["joints"]:
        par = q["parent"]
        bones.append((par + off if par >= 0 else -1, q["std"] if q.get("std") is not None else -1, q["init_key"]))

verts, by_mat = [], {}
for s in j["series"]:
    ob, bs, n = s["object"], s["bones"], s["count"]
    first = len(verts)
    for v in range(s["first_vertex"], s["first_vertex"] + n):
        r = struct.unpack_from("<6f4B2f4B", vtx, 40 * v)
        sl = r[12]
        b = bs[sl] if sl < len(bs) else (bs[0] if bs else 0)
        b = base[ob] + (b if b < len(j["objects"][ob]["joints"]) else 0)
        # rgba stays on the PS2 scale (0x80 = 1.0), as in the test model
        verts.append((r[0:3], r[3:6], r[6:10], r[10:12], b))
    for t in tris(s["prim_type"], n):
        p = [verts[first + x][0] for x in t]
        if len({tuple(x) for x in p}) < 3:
            continue
        by_mat.setdefault((s["pass"], s["material"]), []).extend(first + x for x in t)

mats = j["materials"]
order = sorted(by_mat)  # by pass, then material
idx, parts = [], []
for key in order:
    m = key[1]
    parts.append((m, len(idx), len(by_mat[key])))
    idx += by_mat[key]

o = b"SPM2" + bytes([1 if a.bind else 0]) + struct.pack("<f", j["scene_scale"])
if a.bind:
    nm = a.bind.encode()
    o += struct.pack("<H", len(nm)) + nm + struct.pack("<IH", a.size, a.anchor) + struct.pack("<12f", *a.place)
o += struct.pack("<H", len(bones))
for par, std, k in bones:
    o += struct.pack("<hhh6f", par, std, -1, *k)
o += struct.pack("<H", len(mats))
for m in mats:
    o += bytes([0, m["pass"], 255, 1])  # no picture, pass, blend not known, both sides
o += struct.pack("<I", len(verts))
for p, nrm, c, uv, b in verts:
    o += struct.pack("<3f3f4B2fHH", *p, *nrm, *c, *uv, b, 0)
o += struct.pack("<I", len(parts))
for m, f, c in parts:
    o += struct.pack("<HII", m, f, c)
o += struct.pack("<I", len(idx)) + struct.pack("<%dI" % len(idx), *idx)
open(a.out, "wb").write(o)
print(a.id, len(bones), "bones", len(mats), "materials", len(verts), "vertices", len(idx) // 3, "triangles ->", a.out)
