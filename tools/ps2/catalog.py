#!/usr/bin/env python3
"""Каталог моделей PS2 (K-125): все архивы LF → OBJ + текстуры + миниатюры + индекс (только чтение).

    catalog.py [--no-nodes] RAWDIR OUTDIR [ARCHIVE.BIN ...]   по умолчанию все *.BIN из RAWDIR

Части с таблицей узлов (машины, K-184) ставятся по узлам (`g101_nodes.place`); `--no-nodes` — как было.

В OUTDIR/<АРХИВ>/: `NNN.obj` (+`NNN.mtl`, `NNN_K.png` текстуры записи), `NNN_view.png` (три проекции
каркаса); в OUTDIR: `index.csv` (запись, id, вершин, треугольников, серий списков/полос, мусорных,
bbox, картинки) и `sheet_<АРХИВ>.png` (контактный лист миниатюр). Серии вершин собирает
`g101_verts.entry_mesh` (полосы и списки). Без PIL и numpy.
"""
import csv
import glob
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import g101_nodes as gn
import g101_verts as g
import lf

THUMB = 160


def line(img, w, h, x0, y0, x1, y1, col):
    dx, dy = abs(x1 - x0), -abs(y1 - y0)
    sx, sy = (1 if x0 < x1 else -1), (1 if y0 < y1 else -1)
    err = dx + dy
    for _ in range(4 * (w + h)):
        if 0 <= x0 < w and 0 <= y0 < h:
            o = (y0 * w + x0) * 4
            img[o:o + 4] = col
        if x0 == x1 and y0 == y1:
            break
        e2 = 2 * err
        if e2 >= dy:
            err += dy
            x0 += sx
        if e2 <= dx:
            err += dx
            y0 += sy


def view(tris, bb, size=THUMB):
    """Три проекции каркаса (сверху XZ, спереди XY, сбоку ZY) в одну картинку size*3 × size."""
    w, h = size * 3, size
    img = bytearray([255, 255, 255, 255] * w * h)
    proj = [(0, 2), (0, 1), (2, 1)]
    for k, (a, b) in enumerate(proj):
        ext = max(bb[a][1] - bb[a][0], bb[b][1] - bb[b][0], 1e-6)
        sc = (size - 8) / ext
        cx, cy = (bb[a][0] + bb[a][1]) / 2, (bb[b][0] + bb[b][1]) / 2

        def pt(p):
            return (int(k * size + size / 2 + (p[a] - cx) * sc), int(size / 2 - (p[b] - cy) * sc))
        for t in tris[:6000]:
            ps = [pt(v) for v in t]
            for i in range(3):
                line(img, w, h, *ps[i], *ps[(i + 1) % 3], b'\x30\x30\x30\xff')
    return w, h, bytes(img)


def sheet(thumbs, cols, path):
    """Контактный лист: миниатюры (w, h, rgba) по cols в ряд."""
    if not thumbs:
        return
    cw = max(t[0] for t in thumbs)
    ch = max(t[1] for t in thumbs)
    rows = (len(thumbs) + cols - 1) // cols
    W, H = cw * cols, ch * rows
    img = bytearray([200, 200, 200, 255] * W * H)
    for i, (w, h, px) in enumerate(thumbs):
        x0, y0 = (i % cols) * cw, (i // cols) * ch
        for y in range(h):
            o = ((y0 + y) * W + x0) * 4
            img[o:o + w * 4] = px[y * w * 4:(y + 1) * w * 4]
    lf.write_png(path, W, H, bytes(img))


def write_obj(path, tris, mtl=None):
    pts, uvs, nrm, faces, index = [], [], [], [], {}
    for t in tris:
        ids = []
        for f in t:
            key = tuple(round(x, 3) for x in (f[0], f[1], f[2], f[12], f[13]))
            if key not in index:
                index[key] = len(pts) + 1
                pts.append(f[0:3]); uvs.append(f[12:14]); nrm.append(f[4:7])
            ids.append(index[key])
        faces.append(ids)
    with open(path, 'w') as o:
        if mtl:
            o.write('mtllib %s\nusemtl m0\n' % mtl)
        for v in pts: o.write('v %.4f %.4f %.4f\n' % v)
        for t in uvs: o.write('vt %.4f %.4f\n' % (t[0], 1 - t[1]))
        for n in nrm: o.write('vn %.4f %.4f %.4f\n' % n)
        for t in faces: o.write('f ' + ' '.join('%d/%d/%d' % (i, i, i) for i in t) + '\n')
    return len(pts)


def main(argv):
    nodes_on = '--no-nodes' not in argv
    argv = [a for a in argv if a != '--no-nodes']
    raw, out = argv[0], argv[1]
    names = argv[2:] or [os.path.basename(p) for p in sorted(glob.glob(os.path.join(raw, '*.BIN')))]
    os.makedirs(out, exist_ok=True)
    rows = []
    for name in names:
        arc = lf.Archive(open(os.path.join(raw, name), 'rb').read())
        stem = name.rsplit('.', 1)[0]
        d_out = os.path.join(out, stem)
        thumbs = []
        for e in arc.entries:
            d = bytes(e.data)
            tris, nl, ns, bad = g.entry_mesh(d)
            if not tris:
                continue
            if nodes_on:
                tris, rep = gn.place(tris, gn.read_nodes(d))
                if rep['placed']:
                    print('  %s#%d: поставлено частей %d из %d узлов' % (stem, e.index, rep['placed'], rep['nodes']))
            os.makedirs(d_out, exist_ok=True)
            imgs = []
            for im in e.images:
                if im.is_clut:
                    continue
                rgba = lf.to_rgba(im, lf.pair_clut(e, im))
                if rgba:
                    p = '%03d_%d.png' % (e.index, im.index)
                    lf.write_png(os.path.join(d_out, p), im.w, im.h, rgba)
                    imgs.append((p, im.w, im.h))
            mtl = None
            if imgs:
                mtl = '%03d.mtl' % e.index
                open(os.path.join(d_out, mtl), 'w').write('newmtl m0\nmap_Kd %s\n' % imgs[0][0])
            nv = write_obj(os.path.join(d_out, '%03d.obj' % e.index), tris, mtl)
            bb = [(min(v[i] for t in tris for v in t), max(v[i] for t in tris for v in t)) for i in range(3)]
            w, h, px = view(tris, bb)
            lf.write_png(os.path.join(d_out, '%03d_view.png' % e.index), w, h, px)
            thumbs.append((w, h, px))
            rows.append([stem, e.index, hex(e.id), nv, len(tris), nl, ns, bad,
                         ' '.join('%.1f..%.1f' % b for b in bb), ' '.join('%s:%dx%d' % i for i in imgs)])
        sheet(thumbs, 2, os.path.join(out, 'sheet_%s.png' % stem))
        print(stem, 'записей с моделью:', len(thumbs))
    with open(os.path.join(out, 'index.csv'), 'w', newline='') as f:
        w = csv.writer(f)
        w.writerow(['архив', 'запись', 'id', 'вершин', 'треугольников', 'серий-списков', 'серий-полос', 'мусорных', 'bbox x y z', 'картинки'])
        w.writerows(rows)
    print('index.csv:', len(rows), 'моделей')


if __name__ == '__main__':
    main(sys.argv[1:])
