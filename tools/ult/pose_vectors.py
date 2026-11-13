#!/usr/bin/env python3
"""Эталонные позы для У1.10: пары «модель × движение» из sunpetal-ee (edges.tsv: персонаж → запись → модель,
персонаж → движение), кадры 0, 1, середина, последний, дробный n+0,5; ключи и мировые матрицы по gigpose.py.
python3 -I tools/ult/pose_vectors.py КАТАЛОГ_sunpetal-ee ВЫХОД.json [ЧИСЛО_ПАР=20]
Читает только дерево sunpetal-ee, пишет только ВЫХОД."""
import sys, os, json
ee, out = sys.argv[1], sys.argv[2]
npairs = int(sys.argv[3]) if len(sys.argv) > 3 else 20
sys.path.insert(0, os.path.join(ee, "tools"))
import gig, gim, gigpose, wmsg
A = os.path.join(ee, "out/assets")
edges = [l.split("\t") for l in open(A + "/edges.tsv").read().split("\n")[1:] if l]
rec_model = {}
for f, t, k, _ in edges:
    if f.startswith("rec:") and k == "GIG" and t.startswith("model:"):
        rec_model.setdefault(f, t[6:])
chara_rec = {f: t for f, t, k, _ in edges if f.startswith("chara:") and k == "record"}
motions = {}
for f, t, k, _ in edges:
    if f.startswith("chara:") and t.startswith("motion:") and k in ("walk", "idle"):
        motions.setdefault(f, {})[k] = t[7:]
mtsv = {l.split("\t")[0]: l.split("\t")[6] for l in open(A + "/motions.tsv").read().split("\n")[1:] if l}
models = {l.split("\t")[0]: l.split("\t")[6] for l in open(A + "/models.tsv").read().split("\n")[1:] if l}


def load_gim(first):
    f, rest = first.split("#")
    i, off = rest.split("@")
    d = open(f"{ee}/extract/event/{f}", "rb").read()
    s, z = wmsg.lf_entries(d)[int(i)]
    return gim.parse(d[s * 0x800: s * 0x800 + z], int(off, 16))


def load_gig(first):
    # у моделей смещение — от начала файла (asset_graph: rec_o + off)
    f, rest = first.split("#")
    off = rest.split("@")[1]
    return gig.parse(open(f"{ee}/extract/event/{f}", "rb").read(), int(off, 16))


pairs, seen = [], set()
for c in sorted(chara_rec):
    mod = rec_model.get(chara_rec[c])
    for kind in ("walk", "idle"):
        mo = motions.get(c, {}).get(kind)
        if mod and mo and (mod, mo) not in seen:
            seen.add((mod, mo))
            pairs.append((mod, mo, kind))
res = []
for mod, mo, kind in pairs:
    if len(res) >= npairs:
        break
    try:
        g = load_gig(models[mod])
        a = load_gim(mtsv[mo])
    except Exception as e:
        print('пропуск', mod, mo, repr(e)[:80])
        continue
    J = g["objects"][0]["joints"] if g["objects"] else []
    if not J:
        continue
    n = a["range"]
    frames = sorted({0, 1, n // 2, n, n + 0.5 if n else 0.5})
    out_f = []
    for fr in frames:
        keys = gigpose.sample_gim(a, J, fr)
        W = gigpose.world(J, keys=keys)
        out_f.append(dict(frame=fr, keys=[list(k) for k in keys], world=[[x for row in m for x in row] for m in W]))
    res.append(dict(model=mod, motion=mo, kind=kind, frames=out_f))
json.dump(res, open(out, "w"))
print(len(res), "пар →", out)
