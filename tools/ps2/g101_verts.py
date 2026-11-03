"""Разведка K-003: достаёт вершины моделей PS2 из записи архива LF (R*.BIN) и пишет OBJ.

    python3 tools/ps2/g101_verts.py R00.BIN ЗАПИСЬ out.obj

Формат вершины (подтверждён на R00, запись 1): 64 байта, четыре строки по 4 float32:
  (x, y, z, 1.0) позиция; (nx, ny, nz, 1.0) нормаль; (r, g, b, 128.0) цвет 0..255;
  (u, v, 1.0, 0.0) текстурные координаты. Индексов нет: серии вершин — списки или полосы; блоки идут пачками до 51 вершины (17 треугольников). Только чтение.
  Серии не кратные трём — полосы (strips); кратные собираются обоими способами (`run_triangles`).
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


def good(f):
    """Вершина пригодна: нет мусорных значений (UV ~1e33 и т. п.)."""
    return all(math.isfinite(x) and abs(x) < 1e5 for x in f[:3]) and -0.05 <= f[12] <= 8 and -0.05 <= f[13] <= 8


def _cost(tris):
    """Сумма длин рёбер: для верной сборки меньше (рёбра короче)."""
    t = 0.0
    for a, b, c in tris:
        for p, q in ((a, b), (b, c), (c, a)):
            t += math.dist(p[:3], q[:3])
    return t / max(1, len(tris))


def run_triangles(R):
    """Треугольники серии вершин R (список кортежей из 16 float).

    Серии — полосы (strips: 4, 5, 6, 8, 12...) или списки (кратные трём, пачки до 51). Серию,
    кратную трём, собираем обоими способами и берём тот, где рёбра короче."""
    n = len(R)
    strip = [(R[i], R[i + 1], R[i + 2]) if i % 2 == 0 else (R[i + 1], R[i], R[i + 2]) for i in range(n - 2)]
    if n % 3:
        return strip, 'strip'
    lst = [(R[i], R[i + 1], R[i + 2]) for i in range(0, n, 3)]
    return (lst, 'list') if _cost(lst) <= _cost(strip) else (strip, 'strip')


def entry_mesh(d):
    """Меш записи: (треугольники, число серий-списков, число серий-полос, отброшено мусорных)."""
    tris, nl, ns, bad = [], 0, 0, 0
    for s, c in runs(d):
        R = [struct.unpack_from('<16f', d, s + 64 * k) for k in range(c)]
        ts, kind = run_triangles(R)
        if kind == 'list':
            nl += 1
        else:
            ns += 1
        for t in ts:
            if all(good(v) for v in t) and len({tuple(v[:3]) for v in t}) == 3:
                tris.append(t)
            else:
                bad += 1
    return tris, nl, ns, bad


def main():
    arc = lf.Archive(open(sys.argv[1], 'rb').read())
    d = bytes(arc.entries[int(sys.argv[2])].data)
    pts, uvs, nrm, faces, index = [], [], [], [], {}
    tris, nl, ns, degen = entry_mesh(d)
    rs = range(nl + ns)
    for t in tris:
        ids = []
        for f in t:
            key = tuple(round(x, 3) for x in (f[0], f[1], f[2], f[12], f[13]))
            if key not in index:
                index[key] = len(pts) + 1
                pts.append(f[0:3]); uvs.append(f[12:14]); nrm.append(f[4:7])
            ids.append(index[key])
        faces.append(ids)
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
