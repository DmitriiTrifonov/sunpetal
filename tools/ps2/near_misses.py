#!/usr/bin/env python3
"""Near misses: DS textures that got no PS2 replacement, with their best PS2 candidate anyway.

    near_misses.py ROM DISC OUT_DIR --cache INDEX [--report build/game2-report.csv]
                   [--min-side 64] [--mae 12:30] [--max-block 60] [--max-edges 1.2] [--top 60]
                   [--per-sheet 12] [--names FILE]

Reads the report of build-data.py (the rows with an empty source are the unmatched ones), finds each
one's best candidate on the disc exactly as build-data.py does (small copies, then the exact
comparison at the DS size) but keeps it whatever the thresholds say, and writes

    OUT_DIR/near-misses.csv     every unmatched texture: name, rom file, candidate, errors, sizes
    OUT_DIR/sheet-NN.png        DS | PS2 pairs, `--per-sheet` per sheet (3 columns, left to right,
                                top to bottom), for the textures that pass the filters, biggest first
    OUT_DIR/sheets.csv          which pair is where: sheet, position, name, candidate, errors

`--names FILE` makes the sheets from the listed names (one per line, # comments) instead.
The index is read-only here; the sheets are for looking at and are not to be committed.
"""
import argparse
import csv
import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, '..', 'ds'))
sys.path.insert(0, os.path.join(HERE, '..'))
import iso
import lf
import mkpack
import nitro
import spdata
from ndskit import rom as ndsrom

K = 8
CELL = 128


def extra(ds, small, w, h):
    """Two more signs than build-data.py has: the correlation of the luma over the DS texture's opaque
    pixels (blind to tone and contrast, which the PS2 picture changes) and the share of pixels whose
    opacity differs by half or more (a different silhouette)."""
    a, b = mkpack.luma(ds, w, h), mkpack.luma(small, w, h)
    idx = [i for i in range(w * h) if ds[4 * i + 3]]
    n = len(idx) or 1
    ma, mb = sum(a[i] for i in idx) / n, sum(b[i] for i in idx) / n
    va = sum((a[i] - ma) ** 2 for i in idx)
    vb = sum((b[i] - mb) ** 2 for i in idx)
    cov = sum((a[i] - ma) * (b[i] - mb) for i in idx)
    corr = cov / ((va * vb) ** 0.5) if va > 1e-6 and vb > 1e-6 else 0.0
    diff = sum(1 for i in range(w * h) if abs(ds[4 * i + 3] - small[4 * i + 3]) >= 128) / (w * h)
    return corr, diff


def fit(w, h, rgba, cell):
    """Nearest-neighbour scale to fit a cell, composited over a grey checker."""
    s = min(cell / w, cell / h)
    nw, nh = max(1, int(w * s)), max(1, int(h * s))
    out = bytearray(cell * cell * 4)
    for y in range(cell):
        for x in range(cell):
            o = (y * cell + x) * 4
            g = 90 if ((x // 8 + y // 8) & 1) else 130
            out[o:o + 4] = bytes((g, g, g, 255))
    ox, oy = (cell - nw) // 2, (cell - nh) // 2
    for y in range(nh):
        sy = min(h - 1, int(y / s))
        for x in range(nw):
            sx = min(w - 1, int(x / s))
            i = (sy * w + sx) * 4
            a = rgba[i + 3]
            o = ((oy + y) * cell + ox + x) * 4
            for c in range(3):
                out[o + c] = (rgba[i + c] * a + out[o + c] * (255 - a)) // 255
    return out


def sheet(pairs, cols=3, gap=6):
    rows = (len(pairs) + cols - 1) // cols
    pw = 2 * CELL + gap
    W, H = cols * pw + (cols + 1) * gap, rows * CELL + (rows + 1) * gap
    out = bytearray(b'\x20\x20\x20\xff' * (W * H))
    for n, (ds, ps) in enumerate(pairs):
        cx = gap + (n % cols) * (pw + gap)
        cy = gap + (n // cols) * (CELL + gap)
        for k, cell in enumerate((ds, ps)):
            for y in range(CELL):
                o = ((cy + y) * W + cx + k * CELL) * 4
                out[o:o + CELL * 4] = cell[y * CELL * 4:(y + 1) * CELL * 4]
    return W, H, bytes(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument('rom')
    ap.add_argument('disc')
    ap.add_argument('out')
    ap.add_argument('--cache', required=True)
    ap.add_argument('--report', default='build/game2-report.csv')
    ap.add_argument('--min-side', type=int, default=64)
    ap.add_argument('--mae', default='12:30')
    ap.add_argument('--max-block', type=float, default=60.0)
    ap.add_argument('--max-edges', type=float, default=1.2)
    ap.add_argument('--top', type=int, default=60)
    ap.add_argument('--per-sheet', type=int, default=12)
    ap.add_argument('--names')
    a = ap.parse_args()
    t0 = time.time()
    log = lambda s: print(f'[{time.time() - t0:6.0f}s] {s}', file=sys.stderr, flush=True)
    os.makedirs(a.out, exist_ok=True)

    missing = {r['name'] for r in csv.DictReader(open(a.report)) if not r['source']}
    log(f'{len(missing)} unmatched in the report')
    rom = ndsrom.Rom(open(a.rom, 'rb').read())
    items = {}
    for k, path, t, pal in nitro.catalog(rom):
        n = spdata.file_name(k)
        if n in missing and k not in items:
            items[k] = (t.w, t.h, nitro.decode(t, pal), path)
    log(f'{len(items)} decoded')

    disc = iso.Iso(a.disc)
    idx = mkpack.build_index(disc, a.cache)
    seen, ded = set(), []
    for rec in idx:       # as build-data.dedupe
        sig = (rec[3], rec[4], tuple(int(v) // 2 for v in rec[5]))
        if sig not in seen:
            seen.add(sig)
            ded.append(rec)
    buckets = {}
    for rec in ded:
        buckets.setdefault((rec[3], rec[4]), []).append(rec)
    cands = {}
    for k, (w, h, rgba, _) in items.items():
        t = mkpack.thumb(w, h, rgba)
        best = []
        for f in (1, 2, 4, 8):
            for (fl, e, i, pw, ph, pt) in buckets.get((w * f, h * f), ()):
                best.append((sum(abs(p - q) for p, q in zip(t, pt)) / len(t), fl, e, i, f))
        best.sort()
        cands[k] = best[:K]
    need = {}
    for cs in cands.values():
        for d, fl, e, i, f in cs:
            need.setdefault(fl, set()).add((e, i))
    pics = {}
    for fl, wanted in sorted(need.items()):
        for x, im in mkpack.images(disc, fl):
            if (x.index, im.index) in wanted:
                pics[(fl, x.index, im.index)] = (im.w, im.h, lf.to_rgba(im, lf.pair_clut(x, im)))
    log('pictures loaded')

    rows = []     # (key, rec dict, ps2 rgba, pw, ph)
    for k, (w, h, rgba, path) in items.items():
        best = None
        for d, fl, e, i, f in cands[k]:
            pw, ph, prgba = pics[(fl, e, i)]
            err, block, edges = mkpack.compare(rgba, mkpack.shrink(pw, ph, prgba, f), w, h)
            if best is None or err < best[0]:
                best = (err, block, edges, fl, e, i, pw, ph, prgba, f)
        if not best:
            continue
        err, block, edges, fl, e, i, pw, ph, prgba, f = best
        corr, adiff = extra(rgba, mkpack.shrink(pw, ph, prgba, f), w, h)
        rec = dict(name=spdata.file_name(k), rom_file=path, candidate=f'{fl}#{e}.{i}', mae=f'{err:.2f}',
                   worst_block=f'{block:.1f}', edge_ratio=f'{edges:.2f}', ds_w=w, ds_h=h, ps2_w=pw, ps2_h=ph,
                   luma_corr=f'{corr:.2f}', alpha_diff=f'{adiff:.2f}')
        rows.append((k, rec, prgba, rgba))
    rows.sort(key=lambda r: r[1]['name'])
    with open(os.path.join(a.out, 'near-misses.csv'), 'w', newline='') as f:
        wr = csv.DictWriter(f, fieldnames=list(rows[0][1]))
        wr.writeheader()
        for _, rec, _, _ in rows:
            wr.writerow(rec)
    log(f'near-misses.csv: {len(rows)} rows')

    if a.names:
        want = [l.split('#')[0].strip() for l in open(a.names)]
        want = [n for n in want if n]
        by = {r[1]['name']: r for r in rows}
        sel = [by[n] for n in want if n in by]
    else:
        lo, hi = (float(v) for v in a.mae.split(':'))
        sel = [r for r in rows
               if min(r[1]['ds_w'], r[1]['ds_h']) >= a.min_side and lo <= float(r[1]['mae']) <= hi
               and float(r[1]['worst_block']) <= a.max_block and float(r[1]['edge_ratio']) <= a.max_edges]
        sel.sort(key=lambda r: (-r[1]['ds_w'] * r[1]['ds_h'], float(r[1]['mae'])))
        sel = sel[:a.top]
    log(f'{len(sel)} pairs for the sheets')
    with open(os.path.join(a.out, 'sheets.csv'), 'w', newline='') as f:
        wr = csv.writer(f)
        wr.writerow(['sheet', 'pos', 'name', 'rom_file', 'candidate', 'mae', 'worst_block', 'edge_ratio', 'ps2_size'])
        for s in range(0, len(sel), a.per_sheet):
            part = sel[s:s + a.per_sheet]
            pairs = []
            for n, (k, rec, prgba, rgba) in enumerate(part):
                pairs.append((fit(rec['ds_w'], rec['ds_h'], rgba, CELL), fit(rec['ps2_w'], rec['ps2_h'], prgba, CELL)))
                wr.writerow([s // a.per_sheet + 1, n + 1, rec['name'], rec['rom_file'], rec['candidate'], rec['mae'],
                             rec['worst_block'], rec['edge_ratio'], f"{rec['ps2_w']}x{rec['ps2_h']}"])
            w, h, px = sheet(pairs)
            lf.write_png(os.path.join(a.out, f'sheet-{s // a.per_sheet + 1:02d}.png'), w, h, px)
    log('done')


if __name__ == '__main__':
    main()
