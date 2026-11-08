#!/usr/bin/env python3
"""K-110, автоматическая часть: читает .spm обратно, сверяет числа с конвертером, пишет OBJ и PNG (3 вида).
Использование: k110_check.py MANIFEST.json DIR ИМЯ"""
import json, math, os, struct, sys, zlib
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'ps2'))
import mkmodels

man, d, name = sys.argv[1:4]
sw = json.load(open(man))
raw = sw['raw']
verts, idx = mkmodels.convert(sw['swaps'][name], raw)
b = open(f'{d}/{name}.spm', 'rb').read()
assert b[:4] == b'SPM1', 'не SPM1'
o = 5 + (8 + 8 + 1 + 2 + 2 if b[4] else 0)
nv, ni = struct.unpack_from('<II', b, o); o += 8
V = [struct.unpack_from('<8f', b, o + 32 * i) for i in range(nv)]
o += 32 * nv
I = list(struct.unpack_from(f'<{ni}I', b, o))
ok = nv == len(verts) and ni == len(idx) and ni % 3 == 0 and max(I) < nv
print(f'читаем обратно: вершин {nv} (конвертер {len(verts)}), треугольников {ni // 3} (конвертер {len(idx) // 3}), индексы в границах: {max(I) < nv} -> {"OK" if ok else "РАСХОЖДЕНИЕ"}')
with open(f'{d}/{name}.obj', 'w') as f:
    for v in V: f.write('v %.5f %.5f %.5f\n' % v[:3])
    for v in V: f.write('vt %.5f %.5f\n' % v[6:8])
    for v in V: f.write('vn %.5f %.5f %.5f\n' % v[3:6])
    for i in range(0, ni, 3):
        f.write('f ' + ' '.join(f'{I[i+k]+1}/{I[i+k]+1}/{I[i+k]+1}' for k in range(3)) + '\n')

# PNG: три вида (спереди, сбоку, сверху), заливка по нормали, дальние треугольники рисуются первыми
W = H = 360
def view(ax, ay, depth):
    img = [[(24, 24, 28)] * W for _ in range(H)]
    ps = [(v[ax], v[ay]) for v in V]
    mnx, mxx = min(p[0] for p in ps), max(p[0] for p in ps)
    mny, mxy = min(p[1] for p in ps), max(p[1] for p in ps)
    s = (W - 20) / max(mxx - mnx, mxy - mny, 1e-6)
    tris = sorted(range(0, ni, 3), key=lambda i: -sum(V[I[i + k]][depth] for k in range(3)))
    for i in tris:
        pts = [((V[I[i + k]][ax] - mnx) * s + 10, H - 10 - (V[I[i + k]][ay] - mny) * s) for k in range(3)]
        n = [sum(V[I[i + k]][3 + c] for k in range(3)) for c in range(3)]
        ln = math.sqrt(sum(x * x for x in n)) or 1
        lit = 0.35 + 0.65 * abs(n[1] / ln * 0.5 + n[2] / ln * 0.5 + n[0] / ln * 0.3) / 1.3
        col = (int(230 * lit), int(200 * lit), int(160 * lit))
        x0, x1 = int(min(p[0] for p in pts)), int(max(p[0] for p in pts)) + 1
        y0, y1 = int(min(p[1] for p in pts)), int(max(p[1] for p in pts)) + 1
        (ax_, ay_), (bx, by), (cx, cy) = pts
        den = (by - cy) * (ax_ - cx) + (cx - bx) * (ay_ - cy)
        if abs(den) < 1e-9: continue
        for y in range(max(y0, 0), min(y1, H)):
            for x in range(max(x0, 0), min(x1, W)):
                l1 = ((by - cy) * (x - cx) + (cx - bx) * (y - cy)) / den
                l2 = ((cy - ay_) * (x - cx) + (ax_ - cx) * (y - cy)) / den
                if l1 >= 0 and l2 >= 0 and 1 - l1 - l2 >= 0: img[y][x] = col
    return img
vs = [view(0, 1, 2), view(2, 1, 0), view(0, 2, 1)]
rows = b''.join(b'\0' + b''.join(bytes(vs[k][y][x]) for k in range(3) for x in range(W)) for y in range(H))
def chunk(t, c): return struct.pack('>I', len(c)) + t + c + struct.pack('>I', zlib.crc32(t + c))
png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', W * 3, H, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b'')
open(f'{d}/{name}.png', 'wb').write(png)
print(f'OBJ: {d}/{name}.obj, виды (по осям XY | ZY | XZ): {d}/{name}.png')
sys.exit(0 if ok else 1)
