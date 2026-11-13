"""sunpetal-ee export (models/<id>.json + .vtx.bin) -> SPM2 (crates/sunpetal-data/src/spm2.rs).
usage: ee2spm.py MODELS_DIR ID OUT.spm [--bind DS_NAME [--size N] [--anchor MESH] [--place 12 floats]] [--textures ID,ID...] [--pack DIR [--pictures TEXTURES_DIR]]
Without --bind the model is unbound (flags 0).
--pack DIR: the materials' pictures (TEXTURES_DIR/<id>.png, by default the export's `textures` beside MODELS_DIR)
are copied to DIR under the texture pack's names, `WxH_200_<picture>_<palette>.png`, and the materials name
them; without it every material has no picture. The two hashes are the two halves of the export's picture id
(`5d514401b0_d3760673` -> 0000005d514401b0, 00000000d3760673) until the Rust reader hashes the PS2 bytes itself.
Several objects are merged: their bones follow one another, a vertex's bone is the series' bone for its slot."""
import argparse
import json
import os
import shutil
import struct

ap = argparse.ArgumentParser()
ap.add_argument("dir")
ap.add_argument("id")
ap.add_argument("out")
ap.add_argument("--bind")
ap.add_argument("--size", type=int, default=0)
ap.add_argument("--anchor", type=lambda s: int(s, 0), default=0xFFFF)
ap.add_argument("--place", type=float, nargs=12, default=[1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0])
ap.add_argument("--textures", default="", help="keep only series whose material's picture is one of these ids (comma separated)")
ap.add_argument("--pack", help="a texture pack folder the materials' pictures are copied to")
ap.add_argument("--pictures", help="the export's pictures (default: MODELS_DIR/../textures)")
a = ap.parse_args()

j = json.load(open(os.path.join(a.dir, a.id + ".json")))
vtx = open(os.path.join(a.dir, a.id + ".vtx.bin"), "rb").read()


def png_alpha(path):
    """The set of alpha values of an 8-bit RGBA or palette PNG (no PIL here); None when it is another kind."""
    import zlib

    d = open(path, "rb").read()
    pos, idat, trns = 8, b"", None
    while pos < len(d):
        n, t = struct.unpack(">I4s", d[pos : pos + 8])
        body = d[pos + 8 : pos + 8 + n]
        pos += 12 + n
        if t == b"IHDR":
            w, h, bd, ct, _, _, il = struct.unpack(">IIBBBBB", body)
        elif t == b"IDAT":
            idat += body
        elif t == b"tRNS":
            trns = body
    if ct not in (6, 3) or bd != 8 or il:
        return None
    bpp = 4 if ct == 6 else 1
    raw, st, prev, out, o = zlib.decompress(idat), w * bpp, bytearray(w * bpp), set(), 0
    for _ in range(h):
        ft, line, o = raw[o], bytearray(raw[o + 1 : o + 1 + w * bpp]), o + 1 + st
        for i in range(st):
            x = line[i - bpp] if i >= bpp else 0
            y = prev[i]
            z = prev[i - bpp] if i >= bpp else 0
            if ft == 1:
                line[i] = (line[i] + x) & 255
            elif ft == 2:
                line[i] = (line[i] + y) & 255
            elif ft == 3:
                line[i] = (line[i] + (x + y) // 2) & 255
            elif ft == 4:
                pa, pb, pc = abs(y - z), abs(x - z), abs(x + y - 2 * z)
                line[i] = (line[i] + (x if pa <= pb and pa <= pc else y if pb <= pc else z)) & 255
        prev = line
        out.update(line[3::4] if ct == 6 else {(trns[v] if trns and v < len(trns) else 255) for v in set(line)})
    return out


def blend_of(alphas, vertex_alpha):
    """GS blends every model as Cs*As + Cd*(1-As) (ALPHA_1 = 0x44 in all the captures), so the state that tells
    materials apart is where As comes from: 0 solid, 1 cut-out (the picture has holes), 2 see-through."""
    if vertex_alpha < 128 or (alphas and (max(alphas) < 255 or any(0 < x < 255 for x in alphas))):
        return 2
    return 1 if alphas and 0 in alphas else 0


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

tex = set(x for x in a.textures.split(",") if x)
mats = j["materials"]

verts, by_mat = [], {}
for s in j["series"]:
    if tex and mats[s["material"]].get("texture") not in tex:
        continue
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
used = {m for m, _, _ in parts}
pics = a.pictures or os.path.join(a.dir, "..", "textures")
copied = 0
vmin = {}
for m, f, c in parts:
    vmin[m] = min([vmin.get(m, 128)] + [verts[x][2][3] for x in idx[f : f + c]])
blends = []
for i, m in enumerate(mats):
    t = m.get("texture")
    pa = png_alpha(os.path.join(pics, t + ".png")) if t and os.path.exists(os.path.join(pics, t + ".png")) else None
    blends.append(blend_of(pa, vmin.get(i, 128)))
for i, m in enumerate(mats):
    t = m.get("texture")
    src = os.path.join(pics, t + ".png") if t else None
    if a.pack and i in used and src and os.path.exists(src) and len(t.split("_")) == 2:
        with open(src, "rb") as f:
            head = f.read(24)
        w, h = struct.unpack(">II", head[16:24])
        data, pal = (int(x, 16) if x != "-" else 0 for x in t.split("_"))  # "-": a picture with no palette
        os.makedirs(a.pack, exist_ok=True)
        dst = os.path.join(a.pack, "%dx%d_200_%016x_%016x.png" % (w, h, data, pal))
        if not os.path.exists(dst):
            shutil.copyfile(src, dst)
            copied += 1
        o += bytes([1]) + struct.pack("<QQHH", data, pal, w, h)
    else:
        o += bytes([0])
    o += bytes([m["pass"], blends[i], 1])  # pass, blend, both sides
o += struct.pack("<I", len(verts))
for p, nrm, c, uv, b in verts:
    o += struct.pack("<3f3f4B2fHH", *p, *nrm, *c, *uv, b, 0)
o += struct.pack("<I", len(parts))
for m, f, c in parts:
    o += struct.pack("<HII", m, f, c)
o += struct.pack("<I", len(idx)) + struct.pack("<%dI" % len(idx), *idx)
open(a.out, "wb").write(o)
print("blend (solid, cut-out, see-through):", [blends.count(k) for k in range(3)])
print(a.id, len(bones), "bones", len(mats), "materials", len(verts), "vertices", len(idx) // 3, "triangles ->", a.out)
if a.pack:
    print(copied, "pictures copied to", a.pack)
