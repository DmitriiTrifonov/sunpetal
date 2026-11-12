#!/usr/bin/env python3
"""Which PS2 scene model a DS area model is, and how it sits in the DS model's space (docs/ULTIMATE-IMPORT.md, У4.1).

    align_area.py ROM.nds DS_PATH EXPORT_DIR [--top N] [--only ID,ID...] [--textures TEX,TEX...] [--anchor MESH] [--eps E]

DS_PATH: the area's model file in the ROM (`Model/Area/A46/A46_00.nsbmd.z`). EXPORT_DIR: `sunpetal-ee`'s
`out/export/v1`. Every PS2 scene model (the GIG slots of the scenes' Scene3D records) is tried. First as the
same model in the same units and axes (the DS areas were made from the PS2 ones: room 402 is the PS2 model
shifted by its `Base` node's place): the shift is the commonest difference between a DS vertex and a PS2 one,
and the mark is the share of DS vertices with a PS2 vertex within `--eps` (0.3) of them; the outdoor areas were
modelled anew for the DS in the same place (the vertices differ), so the share within a wider distance (a fiftieth of the area's size, 3 units at most) is a second mark.
If no model passes (half the vertices, or three quarters within 3), the old way: by the proportions of its box, then the best few by all 48 axis orders and
signs, with one scale and a shift from the boxes, scored by how much of each cloud lies in cells the other
fills (a rough guess: look at it before use). Prints the candidates and, for the best,
the `--anchor MESH --place ...` line for `ee2spm.py` (rows x, y, z of the 3 x 3, then the shift).

The game draws a mesh's display list under its node's matrix and POSSCALE, and the runtime puts a replacement
through the anchor mesh's matrix: so the place printed leads from the PS2 model's space into the anchor mesh's
display-list units (model space, back through the node's matrix, over `pos_scale`), not into model space. The
anchor is the biggest mesh of those under the commonest node matrix (`--anchor MESH` names another).

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


def ds_points(rom_path, path, anchor=None):
    """The area model's vertices in its own space (the root node's), its name, the anchor mesh, and that
    mesh's matrix: display-list units -> model space (`pos_scale`, then its node's world matrix)."""
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
            if anchor is None:
                # the meshes under the commonest node matrix; of them, the one with the longest display list
                key = lambda d: tuple(round(x, 6) for r in m.nodes[d['node']]['world'] for x in r)
                groups = {}
                for d in m.draws:
                    groups.setdefault(key(d), []).append(d)
                common = max(groups.values(), key=len)
                anchor = max(common, key=lambda d: m.shapes[d['shape']]['dl'][1])['shape']
            node = next((d['node'] for d in m.draws if d['shape'] == anchor), None)
            if node is None:
                raise SystemExit(f'mesh {anchor} is not drawn by the model')
            w = m.nodes[node]['world']
            return m.name, pts, anchor, [[x * m.pos_scale for x in r] for r in w[:3]] + [list(w[3])]
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
            x, y, z = struct.unpack_from('<3f', raw, 40 * i)
            slot = raw[40 * i + 36]
            bone = s['bones'][slot] if slot < len(s['bones']) else s['bones'][0]
            r, t = ws[bone] if bone < len(ws) else ws[0]
            pts.append(tuple((r[a][0] * x + r[a][1] * y + r[a][2] * z + t[a]) * k for a in range(3)))
    return pts


def into_mesh(rows, shift, mesh):
    """A place into model space -> the same place into a mesh's display-list units (`mesh`: units -> model, row vectors)."""
    a = mesh[:3]
    det = sum(a[0][i] * (a[1][(i + 1) % 3] * a[2][(i + 2) % 3] - a[1][(i + 2) % 3] * a[2][(i + 1) % 3]) for i in range(3))
    if abs(det) < 1e-12:
        raise SystemExit("the anchor mesh's matrix cannot be turned back")
    inv = [[(a[(j + 1) % 3][(i + 1) % 3] * a[(j + 2) % 3][(i + 2) % 3] - a[(j + 1) % 3][(i + 2) % 3] * a[(j + 2) % 3][(i + 1) % 3]) / det
            for j in range(3)] for i in range(3)]
    # v = (p - t) * inv, p = rows * q + shift
    r2 = [[sum(inv[i][j] * rows[i][c] for i in range(3)) for c in range(3)] for j in range(3)]
    s2 = [sum(inv[i][j] * (shift[i] - mesh[3][i]) for i in range(3)) for j in range(3)]
    return r2, s2


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


def cells(ps, size):
    grid = {}
    for p in ps:
        grid.setdefault((int(p[0] // size), int(p[1] // size), int(p[2] // size)), []).append(p)
    return grid


def near_share(ds, ps, eps, shift=(0.0, 0.0, 0.0), grid=None):
    """The share of the points `ds` with a point of `ps` + `shift` within `eps`, and the middle distance of those
    (`grid`: `cells(ps, eps)`, to save making it again)."""
    grid = grid or cells(ps, eps)
    found = []
    for d in ds:
        q = (d[0] - shift[0], d[1] - shift[1], d[2] - shift[2])
        c = (int(q[0] // eps), int(q[1] // eps), int(q[2] // eps))
        best = min((sum((p[i] - q[i]) ** 2 for i in range(3))
                    for dx in (-1, 0, 1) for dy in (-1, 0, 1) for dz in (-1, 0, 1)
                    for p in grid.get((c[0] + dx, c[1] + dy, c[2] + dz), ())), default=None)
        if best is not None and best <= eps * eps:
            found.append(best ** 0.5)
    found.sort()
    return len(found) / max(1, len(ds)), (found[len(found) // 2] if found else 0.0)


def loose_eps(ds, eps):
    """The second, wider mark's distance for a DS cloud: a fiftieth of its middle part's size, from `eps` to 3
    (in a room of ten units everything is within 3 of everything)."""
    lo, hi = box(ds, 0.1)
    return min(3.0, max(eps, 0.02 * max(hi[i] - lo[i] for i in range(3))))


def same_model(ds, ps, eps, sample=120, step=0.5, loose=None, least=0.0):
    """The PS2 cloud as the DS one but for a shift: (share of DS vertices matched within `eps`, middle distance,
    shift, share within `loose`). The shift is the best of no shift and the commonest differences between a DS
    vertex and a PS2 one (the commonest alone may be a dense clump of PS2 vertices, not the model), by the
    share within `loose`; below `least` there the model is given up early."""
    ds, ps = sorted(set(ds)), sorted(set(ps))
    if not ds or not ps:
        return 0.0, 0.0, [0.0, 0.0, 0.0], 0.0
    votes = {}
    for d in thin(ds, sample):
        for p in ps:
            k = (round((d[0] - p[0]) / step), round((d[1] - p[1]) / step), round((d[2] - p[2]) / step))
            votes[k] = votes.get(k, 0) + 1
    tops = sorted(votes, key=votes.get, reverse=True)[:6]
    loose = loose or loose_eps(ds, eps)
    wide, tight = cells(ps, loose), cells(ps, step)
    few = thin(ds, 200)
    # a candidate's mark: the exact share, or the wide one less a quarter (the same vertices before a likeness)
    mark = lambda c: max(near_share(few, ps, step, c, tight)[0], near_share(few, ps, loose, c, wide)[0] - 0.25)
    quick, shift = max((mark(c), c) for c in [(0.0, 0.0, 0.0)] + [tuple(k[i] * step for i in range(3)) for k in tops])
    if quick < least - 0.25:
        return 0.0, 0.0, list(shift), 0.0
    exact_start = near_share(few, ps, step, shift, tight)[0] >= 0.3
    shift = list(shift)
    for r in ((step, step, step) if exact_start else (loose, step, step)):
        # the shift made exact: the middle of the differences of the pairs it brings together
        grid = wide if r == loose else tight
        diffs = []
        for d in ds:
            q = (d[0] - shift[0], d[1] - shift[1], d[2] - shift[2])
            c = (int(q[0] // r), int(q[1] // r), int(q[2] // r))
            best = min(((sum((p[i] - q[i]) ** 2 for i in range(3)), p)
                        for dx in (-1, 0, 1) for dy in (-1, 0, 1) for dz in (-1, 0, 1)
                        for p in grid.get((c[0] + dx, c[1] + dy, c[2] + dz), ())), default=None)
            if best is not None and best[0] <= r * r:
                diffs.append([d[i] - best[1][i] for i in range(3)])
        if len(diffs) < 10:
            continue
        shift = [sorted(x[i] for x in diffs)[len(diffs) // 2] for i in range(3)]
    share, mid = near_share(ds, ps, eps, shift)
    if len(diffs) >= 10:
        # the same vertices give one difference over and over: the commonest (to 1/32, the DS's step), if it matches more
        exact = []
        for i in range(3):
            count = {}
            for x in diffs:
                k = round(x[i] * 32)
                count[k] = count.get(k, 0) + 1
            k = max(count, key=count.get)
            close = [x[i] for x in diffs if abs(x[i] * 32 - k) <= 1.5]
            exact.append(sum(close) / len(close))
        share2, mid2 = near_share(ds, ps, eps, exact)
        if share2 > share:
            share, mid, shift = share2, mid2, exact
    return share, mid, shift, near_share(ds, ps, loose, shift, wide)[0]


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
    anchor = int(argv[argv.index('--anchor') + 1], 0) if '--anchor' in argv else None
    name, ds, anchor, mesh = ds_points(rom, path, anchor)
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
    eps = float(argv[argv.index('--eps') + 1]) if '--eps' in argv else 0.3

    def out_place(rows, shift):
        print('into model space: ' + ' '.join(f'{v:.6g}' for v in [x for r in rows for x in r] + shift))
        rows, shift = into_mesh(rows, shift, mesh)
        print(f'--anchor {anchor} --place ' + ' '.join(f'{v:.6g}' for v in [x for r in rows for x in r] + shift))

    same = []
    for mid in (only or sorted(where)):
        try:
            ps = ps2_points(export, mid, tex)
        except (OSError, KeyError, IndexError, struct.error):
            continue
        if len(ps) >= 30:
            same.append((same_model(ds, ps, eps), mid))
    same.sort(reverse=True)
    same.sort(key=lambda c: (max(c[0][0], c[0][3] - 0.25), c[0][0]), reverse=True)
    for (share, dist, shift, wide), mid in same[:top]:
        print(f'  {mid} the same model: {share:.3f} of the DS vertices within {eps} (middle {dist:.3f}), {wide:.3f} within the wider distance, shift {[round(x, 3) for x in shift]}')
    if same and (same[0][0][0] >= 0.5 or same[0][0][3] >= 0.75):
        (share, dist, shift, wide), mid = same[0]
        nxt = f' (next {same[1][0][0]:.3f})' if len(same) > 1 else ''
        print(f'best: {mid}, the same model shifted: {share:.3f}{nxt}')
        out_place([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], shift)
        return 0
    print('no model is the DS one shifted; by the boxes (a rough guess):')
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
        out_place(rows, shift)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
