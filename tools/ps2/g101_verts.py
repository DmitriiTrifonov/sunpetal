"""Разведка K-003: достаёт вершины моделей PS2 из записи архива LF (R*.BIN) и пишет OBJ.

    python3 tools/ps2/g101_verts.py R00.BIN ЗАПИСЬ out.obj

Формат вершины (подтверждён на R00, запись 1): 64 байта, четыре строки по 4 float32:
  (x, y, z, 1.0) позиция; (nx, ny, nz, 1.0) нормаль; (r, g, b, 128.0) цвет 0..255;
  (u, v, 1.0, 0.0) текстурные координаты. Индексов нет: каждые три вершины подряд —
  треугольник; блоки идут пачками до 51 вершины (17 треугольников). Только чтение.
"""
import math
import struct
import sys

sys.path.insert(0, __file__.rsplit('/', 1)[0])
import lf


def is_vertex(d, o):
    if o + 64 > len(d):
        return False
    f = struct.unpack_from('<16f', d, o)
    return (f[3] == 1.0 and f[7] == 1.0 and f[11] == 128.0 and f[15] == 0.0
            and all(math.isfinite(x) for x in f) and all(abs(f[i]) < 1e5 for i in (0, 1, 2)))


def runs(d):
    """Непрерывные массивы вершин: список (смещение, число вершин)."""
    out, seen = [], set()
    for s in range(0, len(d) - 64, 4):
        if s in seen or not is_vertex(d, s):
            continue
        o, c = s, 0
        while is_vertex(d, o):
            seen.add(o)
            o += 64
            c += 1
        out.append((s, c))
    return out


def main():
    arc = lf.Archive(open(sys.argv[1], 'rb').read())
    d = bytes(arc.entries[int(sys.argv[2])].data)
    pts, uvs, nrm, faces, index, degen = [], [], [], [], {}, 0
    rs = [(s, c) for s, c in runs(d) if c % 3 == 0]
    for s, c in rs:
        ids = []
        for k in range(c):
            f = struct.unpack_from('<16f', d, s + 64 * k)
            key = tuple(round(x, 3) for x in (f[0], f[1], f[2], f[12], f[13]))
            if key not in index:
                index[key] = len(pts) + 1
                pts.append(f[0:3]); uvs.append(f[12:14]); nrm.append(f[4:7])
            ids.append(index[key])
        for i in range(0, c, 3):
            t = ids[i:i + 3]
            if len(set(t)) < 3:
                degen += 1
            else:
                faces.append(t)
    with open(sys.argv[3], 'w') as o:
        for v in pts: o.write('v %.4f %.4f %.4f\n' % v)
        for t in uvs: o.write('vt %.4f %.4f\n' % (t[0], 1 - t[1]))
        for n in nrm: o.write('vn %.4f %.4f %.4f\n' % n)
        for t in faces: o.write('f ' + ' '.join('%d/%d/%d' % (i, i, i) for i in t) + '\n')
    bb = [(min(v[i] for v in pts), max(v[i] for v in pts)) for i in range(3)]
    print('массивов', len(rs), 'вершин', len(pts), 'треугольников', len(faces), 'вырожденных', degen)
    print('bbox', [(round(a, 1), round(b, 1)) for a, b in bb])


if __name__ == '__main__':
    main()
