#!/usr/bin/env python3
"""Pose check (У3.5): bone matrices of a frame the game draws against `nsbca.world_matrices`.

    check_pose.py ROM.nds SAVE.sav SCENE [--press UP] [--model ch_01_sumio_anim] [--advance N]

Runs `sunpetal-debug` to SCENE (free walk), reads the hero's current motion (`motions`, file by SHA-1 as in
anim_probe.py) and `batches` (each run of polygons: model, slot of the last MTX_RESTORE and the position matrix
at the batch). The position matrix is P = W_node * K with the same K (object, view, POSSCALE) for every bone of
a model, so P_a * P_b^-1 = W_a * W_b^-1: K drops out. The check compares those products for the bone pairs of
the hero against the matrices computed from the file at the frame the game holds (and one frame before).
Prints the largest deviations in 20.12 units. Criterion: the best frame's deviation over neighbouring bones
< 100 (the game rounds its 1.12 matrices; the plan's `< 2` is not reachable). ALL=1 scans every frame of the motion.
"""
import argparse
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, '..'))
import anim_probe as A
import nsbca
import nsbmd
from ndskit import rom as ndsrom


def inv4(m):
    n = 4
    a = [list(map(float, m[i])) + [1.0 if i == j else 0.0 for j in range(n)] for i in range(n)]
    for c in range(n):
        p = max(range(c, n), key=lambda r: abs(a[r][c]))
        a[c], a[p] = a[p], a[c]
        d = a[c][c]
        a[c] = [x / d for x in a[c]]
        for r in range(n):
            if r != c:
                f = a[r][c]
                a[r] = [x - f * y for x, y in zip(a[r], a[c])]
    return [row[n:] for row in a]


def mul4(a, b):
    return [[sum(a[i][k] * b[k][j] for k in range(4)) for j in range(4)] for i in range(4)]


def ext(w):
    """4 x 3 row-vector matrix -> 4 x 4."""
    return [list(w[i]) + [1.0 if i == 3 else 0.0] for i in range(4)]


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('rom')
    ap.add_argument('save')
    ap.add_argument('scene')
    ap.add_argument('--press', default='')
    ap.add_argument('--model', default='ch_01_sumio_anim')
    ap.add_argument('--advance', type=int, default=3, help='frames after the keys, before the reading')
    args = ap.parse_args(argv)
    rom = ndsrom.Rom(open(args.rom, 'rb').read())
    index = A.rom_index(rom)
    files = {os.path.basename(n): b for n, b in nsbca.rom_files(rom)}
    model = None
    for _, b in nsbmd.rom_models(rom):
        try:
            for m in nsbmd.parse(b):
                if m.name == args.model and model is None:
                    model = m
        except (nsbmd.NsbmdError, IndexError):
            pass
    if model is None:
        print('model %s not in the ROM' % args.model, file=sys.stderr)
        return 1
    r = A.Repl(args.rom, args.save, args.scene, auto=True)
    try:
        r.cmd('skip 100')
        r.cmd('advance 20')
        if args.press:
            r.cmd('press %s %d' % (args.press, 10 + args.advance))
        else:
            r.cmd('advance %d' % args.advance)
        sets = r.cmd('motions')['motions']
        rows = [x for x in r.cmd('batches')['meshes'] if x['model'] == args.model and x['slot'] != 255]
        ram = r.peek(A.RAM, A.RAM_SIZE)
    finally:
        r.close()
    hero = next((s for s in sets if s['record_field'] == '0228FE18'), sets[0] if sets else None)
    if hero is None or hero['current_object'] is None:
        print('no motion set of the hero', file=sys.stderr)
        return 1
    cur = int(hero['current_object'], 16)
    name, frame = next((n, f) for o, n, f, _ in A.objects(ram, index) if o == cur)
    print('motion %s frame %.2f, %d bone batches' % (name, frame / 4096, len(rows)))
    stored, _ = model.stack_nodes()
    anim = nsbca.parse(files[name])
    anim = anim[0] if isinstance(anim, list) else anim
    pos = {}
    for x in rows:
        pos.setdefault(x['slot'], [x['pos'][4 * i:4 * i + 4] for i in range(4)])
    slots = sorted(pos)
    best = None
    for f in ([x / 2 for x in range(0, 2 * anim['frames'])] if os.environ.get('ALL') else [x / 2 for x in range(max(0, 2 * int(frame / 4096) - 6), 2 * int(frame / 4096) + 5)]):
        if f < 0:
            continue
        w = [ext(m) for m in nsbca.world_matrices(model, files[name], anim, f, lerp=True)]
        worst = wrot = wtr = wadj = 0.0
        for a in slots:
            for b in slots:
                if a >= b or a not in stored or b not in stored:
                    continue
                got = mul4(pos[a], inv4(pos[b]))
                want = mul4(w[stored[a]], inv4(w[stored[b]]))
                rot = max(abs(got[i][j] - want[i][j]) for i in range(3) for j in range(3)) * 4096
                tr = max(abs(got[3][j] - want[3][j]) for j in range(3)) * 4096
                wrot, wtr = max(wrot, rot), max(wtr, tr)
                worst = max(worst, rot, tr)
                if stored[b] != stored[a] and (model.nodes[stored[b]]['parent'] == stored[a] or model.nodes[stored[a]]['parent'] == stored[b]):
                    wadj = max(wadj, rot, tr)
                if os.environ.get('DBG') and model.nodes[stored[b]]['parent'] == stored[a] or os.environ.get('DBG') and model.nodes[stored[a]]['parent'] == stored[b]:
                    print('  adj', model.nodes[stored[a]]['name'], model.nodes[stored[b]]['name'], 'rot %.1f tr %.1f' % (rot, tr))
        print('frame %.1f: worst %.1f (rotation %.1f, translation %.1f) neighbours %.1f over %d slots' % (f, worst, wrot, wtr, wadj, len(slots)))
        best = wadj if best is None else min(best, wadj)
    return 0 if best is not None and best < 100 else 1


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
