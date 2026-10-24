#!/usr/bin/env python3
"""Пакет текстур из PS2-диска: сопоставляет выгрузку DS с картинками PS2.

    mkpack.py DISC.iso DUMP_DIR OUT [--max-mae 12] [--cache FILE]

DUMP_DIR — результат `--dump-textures` (PNG с ключами в именах). Для каждой текстуры DS
ищется картинка PS2 с той же раскладкой (в 1, 2, 4 или 8 раз больше): сначала по
уменьшенным копиям, потом точная проверка в разрешении DS. Найденная картинка PS2 в полном
размере пишется в OUT_DIR под именем DS-текстуры; рендерер подставляет её по ключу.
Если OUT заканчивается на `.spdata`, пишется файл данных (tools/ps2/spdata.py) вместо папки.
Отчёт: report.csv рядом с OUT (имя, источник, ошибка; без совпадения — пустой источник).
"""
import argparse
import os
import pickle
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import iso
import lf
import spdata

G = 12            # сетка уменьшенной копии
SHAPES = (0x00, 0x01, 0x02, 0x0A, 0x13, 0x14)


def thumb(w, h, rgba):
    t = []
    for gy in range(G):
        for gx in range(G):
            x0, x1 = gx * w // G, max(gx * w // G + 1, (gx + 1) * w // G)
            y0, y1 = gy * h // G, max(gy * h // G + 1, (gy + 1) * h // G)
            sx, sy = max(1, (x1 - x0) // 4), max(1, (y1 - y0) // 4)
            acc, n = [0, 0, 0], 0
            for y in range(y0, y1, sy):
                for x in range(x0, x1, sx):
                    o = (y * w + x) * 4
                    acc[0] += rgba[o]
                    acc[1] += rgba[o + 1]
                    acc[2] += rgba[o + 2]
                    n += 1
            t.extend(v / n for v in acc)
    return t


def lf_files(disc):
    out = []
    def walk(d, prefix):
        for e in disc.entries(d):
            path = prefix + '/' + e['name']
            if e['dir']:
                walk(e, path)
            elif e['name'].upper().endswith('.BIN') and 'MOVIE' not in path and '/PCM' not in path \
                    and disc.read(e['lba'], 2) == b'LF':
                out.append(path.lstrip('/'))
    walk(disc.find('DATA'), 'DATA')
    return sorted(out)


def images(disc, path):
    e = disc.find(path)
    for x in lf.Archive(disc.read(e['lba'], e['size'])).entries:
        for im in x.images:
            if not im.is_clut and im.psm in SHAPES:
                yield x, im


def build_index(disc, cache):
    if cache and os.path.exists(cache):
        return pickle.load(open(cache, 'rb'))
    idx = []
    for f in lf_files(disc):
        n = len(idx)
        for x, im in images(disc, f):
            if im.w >= 8 and im.h >= 8:
                rgba = lf.to_rgba(im, lf.pair_clut(x, im))
                idx.append((f, x.index, im.index, im.w, im.h, thumb(im.w, im.h, rgba)))
        print(f'{f}: {len(idx) - n} картинок', file=sys.stderr, flush=True)
    if cache:
        pickle.dump(idx, open(cache, 'wb'))
    return idx


def shrink(w, h, rgba, k):
    """Уменьшение в k раз усреднением блоков."""
    if k == 1:
        return rgba
    nw, nh = w // k, h // k
    out = bytearray(nw * nh * 4)
    for y in range(nh):
        for x in range(nw):
            acc = [0, 0, 0, 0]
            for j in range(k):
                for i in range(k):
                    o = ((y * k + j) * w + x * k + i) * 4
                    for c in range(4):
                        acc[c] += rgba[o + c]
            out[(y * nw + x) * 4:(y * nw + x) * 4 + 4] = bytes(v // (k * k) for v in acc)
    return bytes(out)


def mae(a, b):
    """Средняя ошибка по RGB там, где в DS-текстуре есть пиксель."""
    s = n = 0
    for o in range(0, len(a), 4):
        if a[o + 3]:
            s += abs(a[o] - b[o]) + abs(a[o + 1] - b[o + 1]) + abs(a[o + 2] - b[o + 2])
            n += 3
    return s / n if n else 1e9


def luma(rgba, w, h):
    return [(rgba[4 * i] * 299 + rgba[4 * i + 1] * 587 + rgba[4 * i + 2] * 114) // 1000 for i in range(w * h)]


def compare(ds, small, w, h):
    """How well the PS2 picture (shrunk to the DS texture's size) matches the DS texture: the mean
    colour error over the texture's opaque pixels (`mae`), the worst mean error of any of 4 x 4
    blocks (a different layout shows there), and how far the edges (luma gradients) are from the
    DS texture's own, in proportion to the edges it has (a different layout puts them elsewhere).
    Returns (mae, worst_block, edge_ratio)."""
    err = mae(ds, small)
    blocks = []
    bw, bh = max(1, w // 4), max(1, h // 4)
    for by in range(0, h, bh):
        for bx in range(0, w, bw):
            s = n = 0
            for y in range(by, min(by + bh, h)):
                for x in range(bx, min(bx + bw, w)):
                    o = (y * w + x) * 4
                    if ds[o + 3]:
                        s += abs(ds[o] - small[o]) + abs(ds[o + 1] - small[o + 1]) + abs(ds[o + 2] - small[o + 2])
                        n += 3
            if n:
                blocks.append(s / n)
    a, b = luma(ds, w, h), luma(small, w, h)
    gd = ge = n = 0
    for y in range(h - 1):
        for x in range(w - 1):
            i = y * w + x
            if ds[4 * i + 3]:
                for j in (i + 1, i + w):
                    gd += abs((a[j] - a[i]) - (b[j] - b[i]))
                    ge += abs(a[j] - a[i])
                    n += 1
    ratio = (gd / n) / ((ge / n) + 2.0) if n else 0.0
    return err, max(blocks) if blocks else 0.0, ratio


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument('iso')
    ap.add_argument('dump')
    ap.add_argument('out')
    ap.add_argument('--max-mae', type=float, default=12.0)
    ap.add_argument('--cache')
    a = ap.parse_args()
    disc = iso.Iso(a.iso)
    idx = build_index(disc, a.cache)
    as_file = a.out.endswith('.spdata')
    out_dir = os.path.dirname(os.path.abspath(a.out)) if as_file else a.out
    os.makedirs(out_dir, exist_ok=True)
    packed = []
    loaded = {}      # путь -> {(entry, image): rgba}
    report = []
    for name in sorted(n for n in os.listdir(a.dump) if n.endswith('.png')):
        w, h, rgba = lf.read_png(os.path.join(a.dump, name))
        t = thumb(w, h, rgba)
        cand = []
        for (f, e, i, pw, ph, pt) in idx:
            if pw % w == 0 and ph % h == 0 and pw // w == ph // h and pw // w in (1, 2, 4, 8):
                cand.append((sum(abs(p - q) for p, q in zip(t, pt)) / len(t), f, e, i, pw // w))
        cand.sort()
        best = None
        for _, f, e, i, k in cand[:12]:
            if f not in loaded:
                loaded = {f: {(x.index, im.index): (im.w, im.h, lf.to_rgba(im, lf.pair_clut(x, im)))
                              for x, im in images(disc, f)}}
            pw, ph, prgba = loaded[f][(e, i)]
            err = mae(rgba, shrink(pw, ph, prgba, k))
            if best is None or err < best[0]:
                best = (err, f, e, i, pw, ph, prgba)
        if best and best[0] <= a.max_mae:
            if as_file:
                packed.append((spdata.parse_name(name), lf.png_bytes(best[4], best[5], best[6])))
            else:
                lf.write_png(os.path.join(a.out, name), best[4], best[5], best[6])
            report.append((name, f'{best[1]}#{best[2]}.{best[3]}', f'{best[0]:.2f}'))
        else:
            report.append((name, '', f'{best[0]:.2f}' if best else ''))
        print(*report[-1])
    if as_file:
        spdata.write(a.out, [p for p in packed if p[0]])
    with open(os.path.join(out_dir, 'report.csv'), 'w') as f:
        f.write('name,source,mae\n' + ''.join(','.join(r) + '\n' for r in report))
    print(f'найдено {sum(1 for r in report if r[1])} из {len(report)}', file=sys.stderr)


if __name__ == '__main__':
    main()
