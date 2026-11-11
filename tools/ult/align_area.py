#!/usr/bin/env python3
"""Which PS2 scene model a DS area model is, and how it sits in the DS model's space (docs/ULTIMATE-IMPORT.md, У4.1).

    align_area.py ROM.nds DS_PATH EXPORT_DIR [--top N] [--only ID,ID...] [--textures TEX,TEX...]

DS_PATH: the area's model file in the ROM (`Model/Area/A46/A46_00.nsbmd.z`). EXPORT_DIR: `sunpetal-ee`'s
`out/export/v1`. Every PS2 scene model (the GIG slots of the scenes' Scene3D records) is tried: first by the
proportions of its box, then the best few by all 48 axis orders and signs, with one scale and a shift from the
boxes, scored by how much of each cloud lies in cells the other fills. Prints the candidates and, for the best,
the `--place` line for `ee2spm.py` (rows x, y, z of the 3 x 3, then the shift: PS2 model space -> DS model space).

`--textures`: only the PS2 triangles with these pictures (ids of `textures.tsv`) are compared — for a PS2
model that holds more than the DS area (a whole building against one room); the pictures come from the known
DS-PS2 texture pairs.

Reads only; no game data is written anywhere.
"""
import glob
import itertools
import json
import math
import os
import struct
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'ds'))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))
import nsbmd
from ndskit import rom as ndsrom


def ds_points(rom_path, path):
    """The area model's vertices in its own space (the root node's), and its name."""
    rom = ndsrom.Rom(open(rom_path, 'rb').read())
    for p, b in nsbmd.rom_models(rom):
        if p == path or p.endswith(path):
            pts = []
            m = nsbmd.parse(b)[0]
            for d in m.draws:
                s = m.shapes[d['shape']]
                w = m.nodes[d['node']]['world']
                for _, tri in nsbmd.triangles(b, s['dl'][0], s['dl'][1], m.pos_scale):
                    pts.extend(nsbmd.apply(w, v) for v in tri)
            return m.name, pts
    raise SystemExit('no such file in the ROM: ' + path)


def rot(r):
    """Ry . Rz . Rx as rows (the PS2 engine's order), then no shift."""
    (sx, cx), (sy, cy), (sz, cz) = ((math.sin(a), math.cos(a)) for a in r)
    return [[cy * cz, -cy * sz * cx + sy * sx, cy * sz * sx + sy * cx],
            [sz, cz * cx, -cz * sx],
            [-sy * cz, sy * sz * cx + cy * sx, -sy * sz * sx + cy * cx]]


def ps2_points(export, mid, tex=None):
    """A PS2 model's vertices in its own space at rest, in scene units (`tex`: of the materials with these pictures only)."""
    j = json.load(open(f'{export}/models/{mid}.json'))
    raw = open(f'{export}/models/{mid}.vtx.bin', 'rb').read()
    worlds = []
    for o in j['objects']:
        ws = {}

        def world(i, joints=o['joints'], ws=ws):
            if i in ws:
                return ws[i]
            k = joints[i]['init_key']
            r, t = rot(k[3:6]), k[0:3]
            p = joints[i]['parent']
            if p is not None and p >= 0:
                pr, pt = world(p)
                r2 = [[sum(pr[a][c] * r[c][b] for c in range(3)) for b in range(3)] for a in range(3)]
                t = [sum(pr[a][c] * t[c] for c in range(3)) + pt[a] for a in range(3)]
                r = r2
            ws[i] = (r, t)
            return ws[i]
        worlds.append([world(i) for i in range(len(o['joints']))])
    k = j['scene_scale']
    pts = []
    for s in j['series']:
        if tex is not None and j['materials'][s['material']]['texture'] not in tex:
            continue
        ws = worlds[s['object']]
        for i in range(s['first_vertex'], s['first_vertex'] + s['count']):
            x, y, z = struct.unpack_from('<3f', raw, 36 * i)
            slot = raw[36 * i + 32]
            bone = s['bones'][slot] if slot < len(s['bones']) else s['bones'][0]
            r, t = ws[bone] if bone < len(ws) else ws[0]
            pts.append(tuple((r[a][0] * x + r[a][1] * y + r[a][2] * z + t[a]) * k for a in range(3)))
    return pts


def box(pts, cut=0.02):
    """The cloud's box without its outliers: from the `cut` share of the points to 1 - `cut`, on each axis."""
    lo, hi = [], []
    for a in range(3):
        v = sorted(p[a] for p in pts)
        lo.append(v[int(cut * (len(v) - 1))])
        hi.append(v[int((1 - cut) * (len(v) - 1))])
    return lo, hi


def score(a, b, cells=20):
    """How alike two clouds in the same space are: the share of each one's filled cells the other fills too."""
    lo, hi = box(a + b, 0.0)
    step = max(hi[i] - lo[i] for i in range(3)) / cells or 1.0

    def grid(pts):
        return {tuple(int((p[i] - lo[i]) / step) for i in range(3)) for p in pts}
    ga, gb = grid(a), grid(b)
    if not ga or not gb:
        return 0.0
    near = lambda c, g: any((c[0] + dx, c[1] + dy, c[2] + dz) in g for dx in (-1, 0, 1) for dy in (-1, 0, 1) for dz in (-1, 0, 1))
    return (sum(near(c, gb) for c in ga) / len(ga) + sum(near(c, ga) for c in gb) / len(gb)) / 2


def thin(pts, n):
    step = max(1, len(pts) // n)
    return pts[::step]


def fit(ds, ps):
    """The best of the 48 axis orders and signs: (score, rows of the 3 x 3, shift)."""
    dlo, dhi = box(ds)
    dc = [(dlo[i] + dhi[i]) / 2 for i in range(3)]
    dext = [dhi[i] - dlo[i] for i in range(3)]
    plo, phi = box(ps)
    pc = [(plo[i] + phi[i]) / 2 for i in range(3)]
    pext = [phi[i] - plo[i] for i in range(3)]
    best = (0.0, None, None)
    dsub, psub = thin(ds, 4000), thin(ps, 6000)
    for perm in itertools.permutations(range(3)):
        # one scale: the biggest side of the DS box over the PS2 side that goes there
        big = max(range(3), key=lambda i: dext[i])
        if pext[perm[big]] <= 0:
            continue
        k = dext[big] / pext[perm[big]]
        if any(abs(pext[perm[i]] * k - dext[i]) > 0.25 * max(dext) for i in range(3)):
            continue
        for signs in itertools.product((1, -1), repeat=3):
            rows = [[signs[i] * k if c == perm[i] else 0.0 for c in range(3)] for i in range(3)]
            shift = [dc[i] - signs[i] * k * pc[perm[i]] for i in range(3)]
            moved = [tuple(rows[i][perm[i]] * p[perm[i]] + shift[i] for i in range(3)) for p in psub]
            sc = score(dsub, moved)
            if sc > best[0]:
                best = (sc, rows, shift)
    return best


def main(argv):
    if len(argv) < 3:
        print(__doc__)
        return 2
    rom, path, export = argv[0], argv[1], argv[2]
    top = int(argv[argv.index('--top') + 1]) if '--top' in argv else 8
    only = argv[argv.index('--only') + 1].split(',') if '--only' in argv else None
    tex = set(argv[argv.index('--textures') + 1].split(',')) if '--textures' in argv else None
    name, ds = ds_points(rom, path)
    dlo, dhi = box(ds)
    dext = sorted(dhi[i] - dlo[i] for i in range(3))
    print(f'DS {name}: {len(ds)} vertices, box {[round(dhi[i] - dlo[i], 3) for i in range(3)]}')
    where = {}
    for f in sorted(glob.glob(export + '/scenes/*.json')):
        j = json.load(open(f))
        for s in j['scene3d']:
            for slot, mid in s['models'].items():
                if slot == 'GIG':
                    where.setdefault(mid, []).append(f"{j['scene']}#{s['record']}")
    cands = []
    for mid in (only or sorted(where)):
        try:
            ps = ps2_points(export, mid, tex)
        except (OSError, KeyError, IndexError, struct.error) as e:
            print('  skipped', mid, e)
            continue
        if len(ps) < 30:
            continue
        lo, hi = box(ps)
        ext = sorted(hi[i] - lo[i] for i in range(3))
        if ext[2] <= 0:
            continue
        # the boxes' proportions: the two smaller sides over the biggest
        d = sum(abs(ext[i] / ext[2] - dext[i] / dext[2]) for i in range(2))
        cands.append((d, mid, ps))
    cands.sort(key=lambda c: c[0])
    out = []
    for d, mid, ps in cands[:top]:
        sc, rows, shift = fit(ds, ps)
        out.append((sc, mid, d, rows, shift, len(ps)))
        print(f'  {mid} box diff {d:.3f} score {sc:.3f} vertices {len(ps)} in {",".join(where.get(mid, [])[:4])}')
    out.sort(reverse=True)
    if out and out[0][3]:
        sc, mid, d, rows, shift, _ = out[0]
        print(f'best: {mid} score {sc:.3f} (next {out[1][0]:.3f})' if len(out) > 1 else f'best: {mid} score {sc:.3f}')
        print('--place ' + ' '.join(f'{v:.6g}' for v in [x for r in rows for x in r] + shift))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
