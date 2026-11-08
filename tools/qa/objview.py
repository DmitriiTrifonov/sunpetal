#!/usr/bin/env python3
"""Три вида (XY | ZY | XZ) для OBJ без внешних библиотек, по силуэтам и глубине.
objview.py OUT.png DS.obj [PS2.obj]  — с двумя файлами: PS2 подгоняется orient.best_orientation и накладывается на DS
(DS — серый, PS2 — красный полупрозрачный). IoU считается тут же."""
import os, struct, sys, zlib
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'ps2'))
import orient

W = H = 360

def load(p):
    return orient.read_obj(p)

def raster(tris, ax, ay, box, grid=(W, H)):
    mnx, mxx, mny, mxy = box
    s = (W - 20) / max(mxx - mnx, mxy - mny, 1e-9)
    m = [[0] * W for _ in range(H)]
    for t in tris:
        pts = [((p[ax] - mnx) * s + 10, H - 10 - (p[ay] - mny) * s) for p in t]
        (x0, y0), (x1, y1), (x2, y2) = pts
        den = (y1 - y2) * (x0 - x2) + (x2 - x1) * (y0 - y2)
        if abs(den) < 1e-9: continue
        for y in range(max(int(min(y0, y1, y2)), 0), min(int(max(y0, y1, y2)) + 1, H)):
            for x in range(max(int(min(x0, x1, x2)), 0), min(int(max(x0, x1, x2)) + 1, W)):
                a = ((y1 - y2) * (x - x2) + (x2 - x1) * (y - y2)) / den
                b = ((y2 - y0) * (x - x2) + (x0 - x2) * (y - y2)) / den
                if a >= 0 and b >= 0 and 1 - a - b >= 0: m[y][x] = 1
    return m

def png(rows_px, w, h, path):
    raw = b''.join(b'\0' + bytes(c for px in r for c in px) for r in rows_px)
    ch = lambda t, c: struct.pack('>I', len(c)) + t + c + struct.pack('>I', zlib.crc32(t + c))
    open(path, 'wb').write(b'\x89PNG\r\n\x1a\n' + ch(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)) + ch(b'IDAT', zlib.compress(raw)) + ch(b'IEND', b''))

def main(out, ds_path, ps2_path=None):
    ds = load(ds_path)
    ps2 = None
    if ps2_path:
        r = orient.best_orientation(ds, load(ps2_path))
        ps2 = [tuple(orient.apply(r, p) for p in t) for t in load(ps2_path)]
        print('orient:', {k: r[k] for k in r if k in ('perm', 'signs', 'scale', 'offset', 'score', 'ambiguous')})
    pts = [p for t in ds + (ps2 or []) for p in t]
    panels = []
    ious = []
    for ax, ay in ((0, 1), (2, 1), (0, 2)):
        box = (min(p[ax] for p in pts), max(p[ax] for p in pts), min(p[ay] for p in pts), max(p[ay] for p in pts))
        a = raster(ds, ax, ay, box)
        b = raster(ps2, ax, ay, box) if ps2 else None
        img = [[(24, 24, 28)] * W for _ in range(H)]
        both = only_a = only_b = 0
        for y in range(H):
            for x in range(W):
                ia, ib = a[y][x], b[y][x] if b else 0
                if ia and ib: img[y][x] = (210, 150, 90); both += 1
                elif ia: img[y][x] = (170, 170, 180); only_a += 1
                elif ib: img[y][x] = (230, 60, 60); only_b += 1
        if b: ious.append(both / max(both + only_a + only_b, 1))
        panels.append(img)
    png([sum((panels[k][y] for k in range(3)), []) for y in range(H)], W * 3, H, out)
    if ious: print('IoU по трём видам:', ' '.join('%.2f' % v for v in ious))
    print('картинка:', out, '(слева направо: XY, ZY, XZ; серый — только DS, красный — только PS2, оранжевый — оба)' if ps2 else out)

if __name__ == '__main__':
    main(*sys.argv[1:])
