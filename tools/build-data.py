#!/usr/bin/env python3
"""Builds the Sunpetal data file from the player's own copies of the game.

    build-data.py ROM.nds DISC.iso OUT.spdata [--dump DIR] [--cache FILE] [--max-mae 12]

The DS ROM's models and texture archives give every 3D texture the game can draw, with the key the
runtime gives it (tools/ds/nitro.py). Each is matched with the pictures on the PS2 disc (the PS2
version's textures are larger): first by a small copy of the picture, then exactly, at the DS
texture's size. Pairs that match go into the file, as the PS2 picture under the DS texture's key.

Textures the game changes as it runs (character faces, palettes) cannot be worked out from the
files; `--dump DIR` takes the pictures the game wrote with `--dump-textures` and matches those too.
The disc's index (small copies of ~15 000 pictures) takes a quarter of an hour the first time:
`--cache FILE` keeps it.

The file is made from the player's own game and is not to be shared.
"""
import argparse
import os
import re
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, 'ps2'))
sys.path.insert(0, os.path.join(HERE, 'ds'))
import iso
import lf
import mkpack
import nitro
import spdata
from ndskit import rom as ndsrom

K = 8                  # candidates kept per texture from the small copies
NAME = re.compile(r'^(\d+)x(\d+)_(\d)_([0-9a-f]{16})_([0-9a-f]{16})\.png$')


def ds_items(rom, log):
    """{key: (w, h, rgba, palette_known)} from the ROM's files."""
    items = {}
    n = 0
    for k, path, t, pal in nitro.catalog(rom):
        n += 1
        if k in items:
            continue
        items[k] = (t.w, t.h, nitro.decode(t, pal))
        if n % 500 == 0:
            log(f'  decoded {len(items)} textures ({n} pairs)')
    return items


def dump_items(folder, known):
    out = {}
    for name in sorted(os.listdir(folder)):
        m = NAME.match(name)
        if not m:
            continue
        k = (int(m[4], 16), int(m[5], 16), int(m[3]), int(m[1]), int(m[2]))
        if k in known:
            continue
        w, h, rgba = lf.read_png(os.path.join(folder, name))
        out[k] = (w, h, rgba)
    return out


def dedupe(idx):
    """The index without pictures that look the same (the disc holds many copies)."""
    seen, out = set(), []
    for rec in idx:
        sig = (rec[3], rec[4], tuple(int(v) // 2 for v in rec[5]))
        if sig not in seen:
            seen.add(sig)
            out.append(rec)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument('rom')
    ap.add_argument('disc')
    ap.add_argument('out')
    ap.add_argument('--dump')
    ap.add_argument('--cache')
    ap.add_argument('--max-mae', type=float, default=12.0)
    ap.add_argument('--max-block', type=float, default=30.0, help='worst 4x4 block error')
    ap.add_argument('--max-edges', type=float, default=0.65, help='edge difference over edge strength')
    a = ap.parse_args()
    t0 = time.time()
    log = lambda s: print(f'[{time.time() - t0:6.0f}s] {s}', file=sys.stderr, flush=True)

    rom = ndsrom.Rom(open(a.rom, 'rb').read())
    log('reading the DS textures')
    items = ds_items(rom, log)
    log(f'{len(items)} (texture, palette) pairs from the ROM')
    if a.dump:
        extra = dump_items(a.dump, items)
        items.update(extra)
        log(f'{len(extra)} more from the dump')

    disc = iso.Iso(a.disc)
    log('indexing the disc')
    idx = dedupe(mkpack.build_index(disc, a.cache))
    log(f'{len(idx)} distinct pictures on the disc')
    buckets = {}
    for rec in idx:
        buckets.setdefault((rec[3], rec[4]), []).append(rec)

    # phase 1: candidates by the small copies
    cands = {}
    for n, (k, (w, h, rgba)) in enumerate(items.items()):
        t = mkpack.thumb(w, h, rgba)
        best = []
        for f in (1, 2, 4, 8):
            for (fl, e, i, pw, ph, pt) in buckets.get((w * f, h * f), ()):
                d = sum(abs(p - q) for p, q in zip(t, pt)) / len(t)
                best.append((d, fl, e, i, f))
        best.sort()
        cands[k] = best[:K]
    log('candidates chosen')

    # phase 2: exact check, a disc file at a time
    need = {}
    for k, cs in cands.items():
        for d, fl, e, i, f in cs:
            need.setdefault(fl, set()).add((e, i))
    pictures = {}
    for fl, wanted in sorted(need.items()):
        for x, im in mkpack.images(disc, fl):
            if (x.index, im.index) in wanted:
                pictures[(fl, x.index, im.index)] = (im.w, im.h, lf.to_rgba(im, lf.pair_clut(x, im)))
        log(f'  {fl}: {len(wanted)} pictures checked')
    textures, report, best_for = [], [], {}
    for k, (w, h, rgba) in items.items():
        best = None
        for d, fl, e, i, f in cands[k]:
            pw, ph, prgba = pictures[(fl, e, i)]
            small = mkpack.shrink(pw, ph, prgba, f)
            err, block, edges = mkpack.compare(rgba, small, w, h)
            ok = err <= a.max_mae and block <= a.max_block and edges <= a.max_edges
            # the best that passes; else the best of what does not (for the report)
            rank = (0 if ok else 1, err)
            if best is None or rank < best[7]:
                best = (err, fl, e, i, pw, ph, prgba, rank, block, edges)
        name = spdata.file_name(k)
        if best and best[7][0] == 0:
            prgba = best[6]
            if all(rgba[o] == 255 for o in range(3, len(rgba), 4)):
                # an opaque DS texture stays opaque: the PS2 picture's alpha is its own business
                # (holes would show where it is zero)
                prgba = bytearray(prgba)
                prgba[3::4] = b'\xff' * (len(prgba) // 4)
                prgba = bytes(prgba)
            png = lf.png_bytes(best[4], best[5], prgba)
            textures.append((k, png))
            # the same data and size with another palette: this picture, when no exact key is there
            if k[2] != 7:
                wild = (k[0], spdata.ANY_PALETTE, k[2], k[3], k[4])
                if wild not in best_for or best[0] < best_for[wild][0]:
                    best_for[wild] = (best[0], png)
            report.append((name, f'{best[1]}#{best[2]}.{best[3]}', f'{best[0]:.2f}', f'{best[8]:.1f}', f'{best[9]:.2f}'))
        else:
            report.append((name, '', f'{best[0]:.2f}' if best else '', f'{best[8]:.1f}' if best else '', f'{best[9]:.2f}' if best else ''))
    textures += [(k, v[1]) for k, v in best_for.items()]
    spdata.write(a.out, textures)
    with open(os.path.splitext(a.out)[0] + '-report.csv', 'w') as f:
        f.write('name,source,mae,worst_block,edge_ratio\n' + ''.join(','.join(r) + '\n' for r in report))
    done = sum(1 for r in report if r[1])
    log(f'{done} of {len(report)} textures matched; {len(textures)} keys in {a.out}')


if __name__ == '__main__':
    main()
