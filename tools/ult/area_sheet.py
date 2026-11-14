#!/usr/bin/env python3
"""A DS area and the PS2 models at the same place, side by side from above (docs/ULTIMATE-IMPORT.md, У4.2).

    area_sheet.py ROM.nds EXPORT_DIR DS_PATH OUT.png [--cell N] [--most N] [--model ID[,ID..]] [--shift X,Y,Z]

--model shows the named PS2 models (instead of the ones with most vertices); with no --model and no --most the
`ps2` of the area in data/ult/areas.toml is used when there is one. --shift moves the PS2 models by X,Y,Z
(DS units) before drawing (default: the `shift` of the area in areas.toml), for pieces of road that sit shifted against the DS area.

Every cell is the same window (the DS area's box, x across, z down, no shift: the outdoor PS2 world shares
the DS's coordinates). First the DS area whole and then by node (red); then the PS2 models with most
vertices in the window (blue), the DS area in grey under each. The legend (cell -> what it is) is printed
and written beside the picture as OUT.md. Needs `build/ult/ps2_clouds_all.pickle` (`align_all.py --all-models`).
"""
import glob
import json
import os
import pickle
import sys
import tomllib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, '..', 'ds'))
sys.path.insert(0, os.path.join(HERE, '..', 'ps2'))
sys.path.insert(0, os.path.join(HERE, '..'))
import align_area as aa
import lf
import nsbmd
from ndskit import rom as ndsrom


def main(argv):
    if len(argv) < 4:
        print(__doc__)
        return 2
    rom_path, export, path, out = argv[:4]
    T = int(argv[argv.index('--cell') + 1]) if '--cell' in argv else 220
    most = int(argv[argv.index('--most') + 1]) if '--most' in argv else 15
    rom = ndsrom.Rom(open(rom_path, 'rb').read())
    b = next(b for p, b in nsbmd.rom_models(rom) if p.endswith(path))
    m = nsbmd.parse(b)[0]
    nodes = {}
    for d in m.draws:
        s = m.shapes[d['shape']]
        w = m.nodes[d['node']]['world']
        pts = [nsbmd.apply(w, v) for _, tri in nsbmd.triangles(b, s['dl'][0], s['dl'][1], m.pos_scale) for v in tri]
        nodes.setdefault(m.nodes[d['node']]['name'], []).extend(pts)
    ds = [p for pts in nodes.values() for p in pts]
    lo, hi = aa.box(ds, 0.02)
    ext = max(hi[0] - lo[0], hi[2] - lo[2]) * 1.1
    c = [(lo[i] + hi[i]) / 2 for i in range(3)]
    inside = lambda p: abs(p[0] - c[0]) < ext / 2 and abs(p[2] - c[2]) < ext / 2
    with open(os.path.join('build', 'ult', 'ps2_clouds_all.pickle'), 'rb') as f:
        clouds = pickle.load(f)['clouds']
    where = {}
    for f in sorted(glob.glob(export + '/scenes/*.json')):
        j = json.load(open(f))
        for s in j['scene3d']:
            for slot, mid in s['models'].items():
                where.setdefault(mid, []).append(f"{j['scene']}#{s['record']}:{slot}")
    entry = None
    with open(os.path.join(HERE, '..', '..', 'data', 'ult', 'areas.toml'), 'rb') as f:
        entry = next((a for a in tomllib.load(f).get('area', []) if path.endswith(a.get('file', '\0')) or a.get('file', '\0').endswith(path)), None)
    named = argv[argv.index('--model') + 1].split(',') if '--model' in argv else []
    if not named and '--most' not in argv and entry and entry.get('ps2'):
        named = [entry['ps2']]
    shift = [float(v) for v in argv[argv.index('--shift') + 1].split(',')] if '--shift' in argv else (
        entry['shift'] if entry and named == [entry.get('ps2')] and 'shift' in entry else [0, 0, 0])
    if shift != [0, 0, 0]:
        clouds = {k: [(p[0] + shift[0], p[1] + shift[1], p[2] + shift[2]) for p in v] for k, v in clouds.items() if not named or k in named}
    if named:
        missing = [m for m in named if m not in clouds]
        if missing:
            raise SystemExit(f'area_sheet: no cloud for {missing}')
        ps = [(sum(1 for p in clouds[m] if inside(p)), m) for m in named]
    else:
        ps = sorted(((sum(1 for p in pts if inside(p)), mid) for mid, pts in clouds.items()), reverse=True)[:most]
    items = [('DS ' + m.name + ' (whole)', [(ds, (220, 0, 0), 2)])]
    items += [(f'DS node {n} ({len(set(pts))} vertices)', [(ds, (215, 215, 215), 1), (pts, (220, 0, 0), 2)]) for n, pts in sorted(nodes.items(), key=lambda x: -len(x[1]))]
    items += [(f"PS2 {mid} ({n} vertices in the window; {', '.join(where.get(mid, ['not in a scene'])[:3])})", [(ds, (215, 215, 215), 1), (clouds[mid], (0, 0, 220), 1)]) for n, mid in ps if n or named]
    cols = 5
    rows = (len(items) + cols - 1) // cols
    W, H = cols * T, rows * T
    buf = bytearray(b'\xff' * (W * H * 4))
    legend = []
    for i, (name, layers) in enumerate(items):
        ox, oy = (i % cols) * T, (i // cols) * T
        for pts, col, sz in layers:
            for p in pts:
                x, y = int(((p[0] - c[0]) / ext + 0.5) * T), int(((p[2] - c[2]) / ext + 0.5) * T)
                for dx in range(sz):
                    for dy in range(sz):
                        if 1 <= x + dx < T - 1 and 1 <= y + dy < T - 1:
                            o = ((oy + y + dy) * W + ox + x + dx) * 4
                            buf[o:o + 3] = bytes(col)
        legend.append(f'- row {i // cols + 1}, cell {i % cols + 1}: {name}')
    for k in range(1, cols):
        for y in range(H):
            buf[(y * W + k * T) * 4:(y * W + k * T) * 4 + 3] = b'\x99\x99\x99'
    for k in range(1, rows):
        for x in range(W):
            buf[(k * T * W + x) * 4:(k * T * W + x) * 4 + 3] = b'\x99\x99\x99'
    lf.write_png(out, W, H, bytes(buf))
    text = f'# {m.name}: the window is {ext:.0f} units wide, centre x {c[0]:.0f} z {c[2]:.0f}; x across, z down\n\n' + '\n'.join(legend) + '\n'
    with open(os.path.splitext(out)[0] + '.md', 'w') as f:
        f.write(text)
    print(text)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
