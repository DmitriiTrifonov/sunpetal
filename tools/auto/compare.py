#!/usr/bin/env python3
"""Сравнение двух картинок (K-221, docs/AUTONOMY.md А1.8): рядом, карта различий, метрики.

    compare.py A.png B.png [--out FILE.png] [--shift N]

Печатает JSON: размеры, доля отличающихся пикселей, наибольшая разность канала, рамка различий и наилучший сдвиг B
(на сколько пикселей содержимое B уехало вправо/вниз относительно A) в пределах ±N пикселей (по умолчанию 4; ищется по редкой сетке, мера — число несовпавших точек).
`--out` пишет PNG: A | B | карта различий (красное — различия, серое — совпавшее). Код возврата 1, если картинки
различаются. Только стандартная библиотека (чтение PNG — `tools/ps2/lf.py`).
"""
import argparse
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, '..', 'ps2'))
import golden
import lf


def best_shift(a, b, w, h, n, step=4):
    """(dx, dy, доля несовпавших точек сетки) для сдвига B на (dx, dy) относительно A."""
    best = None
    for dy in range(-n, n + 1):
        for dx in range(-n, n + 1):
            bad = tot = 0
            for y in range(max(0, -dy), min(h, h - dy), step):
                for x in range(max(0, -dx), min(w, w - dx), step):
                    i = (y * w + x) * 4
                    j = ((y + dy) * w + (x + dx)) * 4
                    tot += 1
                    if a[i:i + 4] != b[j:j + 4]:
                        bad += 1
            r = bad / tot if tot else 1.0
            if best is None or r < best[2] - 1e-12 or (abs(r - best[2]) < 1e-12 and abs(dx) + abs(dy) < abs(best[0]) + abs(best[1])):
                best = (dx, dy, r)
    return best


def side_by_side(a, b, w, h, dmap):
    out = bytearray()
    for y in range(h):
        out += a[y * w * 4:(y + 1) * w * 4] + b[y * w * 4:(y + 1) * w * 4] + dmap[y * w * 4:(y + 1) * w * 4]
    return 3 * w, h, bytes(out)


def compare(pa, pb, shift=4):
    wa, ha, a = golden.pixels(pa)
    wb, hb, b = golden.pixels(pb)
    res = {'a': [wa, ha], 'b': [wb, hb]}
    if (wa, ha) != (wb, hb):
        res.update(same=False, reason='size')
        return res, None
    d = golden.diff(pa, pb)
    res['same'] = d is None
    res['changed'] = 0 if d is None else d['changed']
    res['fraction'] = res['changed'] / (wa * ha)
    res['max'] = 0 if d is None else d['max']
    res['bbox'] = None if d is None else list(d['bbox'])
    if d is not None and shift:
        dx, dy, r = best_shift(a, b, wa, ha, shift)
        res['shift'] = {'dx': dx, 'dy': dy, 'mismatch': round(r, 4)}
    if d is None:
        g = bytearray()
        for i in range(0, len(a), 4):
            v = (a[i] + a[i + 1] + a[i + 2]) // 9 + 40
            g += bytes((v, v, v, 255))
        return res, side_by_side(a, b, wa, ha, bytes(g))
    return res, side_by_side(a, b, wa, ha, d['map'][2])


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('a')
    ap.add_argument('b')
    ap.add_argument('--out')
    ap.add_argument('--shift', type=int, default=4)
    x = ap.parse_args()
    res, img = compare(x.a, x.b, x.shift)
    if x.out and img:
        lf.write_png(x.out, *img)
    print(json.dumps(res))
    return 0 if res.get('same') else 1


if __name__ == '__main__':
    sys.exit(main())
