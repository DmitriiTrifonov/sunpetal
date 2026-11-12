#!/usr/bin/env python3
"""Does a frame with a PS2 model in place of the DS one show the same scene? (docs/ULTIMATE-IMPORT.md, У4.5)

    compare_shots.py DS.png PS2.png [...more pairs]

Both are `sunpetal-debug --shot` pictures of one moment, without and with `SUNPETAL_ULT_MODELS`. Only the
upper screen (the upper half) is looked at. Prints per pair: `changed` (the share of pixels that differ:
0 means nothing was replaced) and `edges` (of the edge pixels of each picture, the share with an edge pixel
of the other within 2 pixels; the mean of both ways: the same geometry seen by the same camera keeps its
outlines though the pictures on it differ). Exit 1 if a pair has `changed` > 0.02 and `edges` < 0.8 (a wrong placing of room 402 gave 0.62, the right one 0.93).
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'ps2'))
import lf


def upper(path):
    w, h, px = lf.read_png(path)
    h //= 2
    lum = [(px[4 * i] * 3 + px[4 * i + 1] * 6 + px[4 * i + 2]) // 10 for i in range(w * h)]
    return w, h, px[:4 * w * h], lum


def edges(w, h, lum, t=24):
    e = set()
    for y in range(1, h - 1):
        for x in range(1, w - 1):
            i = y * w + x
            if abs(lum[i + 1] - lum[i - 1]) + abs(lum[i + w] - lum[i - w]) > t:
                e.add((x, y))
    return e


def near(a, b, r=2):
    if not a:
        return 1.0
    return sum(any((x + dx, y + dy) in b for dx in range(-r, r + 1) for dy in range(-r, r + 1)) for x, y in a) / len(a)


def compare(ds, ps):
    w, h, pa, la = upper(ds)
    w2, h2, pb, lb = upper(ps)
    if (w, h) != (w2, h2):
        raise SystemExit(f'{ds} and {ps} differ in size')
    changed = sum(pa[4 * i:4 * i + 3] != pb[4 * i:4 * i + 3] for i in range(w * h)) / (w * h)
    ea, eb = edges(w, h, la), edges(w, h, lb)
    return changed, (near(ea, eb) + near(eb, ea)) / 2


def main(argv):
    if len(argv) < 2 or len(argv) % 2:
        print(__doc__)
        return 2
    bad = 0
    for ds, ps in zip(argv[::2], argv[1::2]):
        changed, e = compare(ds, ps)
        ok = changed <= 0.02 or e >= 0.8
        bad += not ok
        print(f'{os.path.basename(ps)}: changed {changed:.3f} edges {e:.3f} {"" if changed > 0.02 else "(not replaced)"}{"" if ok else " MISMATCH"}')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
