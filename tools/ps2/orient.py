#!/usr/bin/env python3
"""Автоподбор преобразования PS2 → DS для 3D-модели (K-127).

Обе модели — один предмет (машина), но в разных системах координат: оси
переставлены и/или перевёрнуты, масштаб и начало другие. Перебираем все 48
знаковых перестановок осей, для каждой приводим габарит PS2 к габариту DS одним
равномерным масштабом по самой длинной оси (центры bbox совмещаются) и меряем
средний IoU силуэтов по трём проекциям (XY, XZ, YZ) на сетке grid×grid.

CLI:  python3 tools/ps2/orient.py DS.obj PS2.obj [--allow-mirror]
"""
import itertools
import math
import sys

_LIMIT = 4000  # порог, после которого треугольники детерминированно прореживаются


def _permutation_parity(p):
    """Знак перестановки: +1 чётная, -1 нечётная."""
    inv = 0
    for i in range(3):
        for j in range(i + 1, 3):
            if p[i] > p[j]:
                inv += 1
    return -1 if inv & 1 else 1


def _build_candidates():
    """Все 48 знаковых перестановок: (perm, signs, mirrored)."""
    out = []
    for perm in itertools.permutations(range(3)):
        for signs in itertools.product((1, -1), repeat=3):
            det = _permutation_parity(perm) * signs[0] * signs[1] * signs[2]
            out.append((perm, signs, det == -1))
    return out


_CANDIDATES = _build_candidates()


def _bbox(points):
    minx = maxx = points[0][0]
    miny = maxy = points[0][1]
    minz = maxz = points[0][2]
    for x, y, z in points:
        if x < minx:
            minx = x
        elif x > maxx:
            maxx = x
        if y < miny:
            miny = y
        elif y > maxy:
            maxy = y
        if z < minz:
            minz = z
        elif z > maxz:
            maxz = z
    return ((minx, maxx), (miny, maxy), (minz, maxz))


def _thin(tris, limit=_LIMIT):
    """Детерминированное прореживание списка треугольников шагом, если их больше limit."""
    if len(tris) <= limit:
        return tris
    step = (len(tris) + limit - 1) // limit
    return tris[::step]


class _Proj:
    """Одна ортогональная проекция: пара осей (a, b) и сетка в общих границах."""

    def __init__(self, a, b, lo, hi, grid):
        self.a, self.b = a, b
        minx, maxx = lo[a], hi[a]
        miny, maxy = lo[b], hi[b]
        self.minx, self.miny = minx, miny
        self.cellx = (maxx - minx) / grid
        self.celly = (maxy - miny) / grid
        self.px = [minx + (i + 0.5) * self.cellx for i in range(grid)]
        self.py = [miny + (i + 0.5) * self.celly for i in range(grid)]
        self.grid = grid

    def raster(self, tris):
        """Заполненные ячейки силуэта списка треугольников (барицентрический тест центров)."""
        a, b = self.a, self.b
        px, py = self.px, self.py
        cellx, celly = self.cellx, self.celly
        minx, miny = self.minx, self.miny
        grid = self.grid
        filled = set()
        for tri in tris:
            x0, y0 = tri[0][a], tri[0][b]
            x1, y1 = tri[1][a], tri[1][b]
            x2, y2 = tri[2][a], tri[2][b]
            tminx = min(x0, x1, x2)
            tmaxx = max(x0, x1, x2)
            tminy = min(y0, y1, y2)
            tmaxy = max(y0, y1, y2)
            cx_lo = math.ceil((tminx - minx) / cellx - 0.5)
            cx_hi = math.floor((tmaxx - minx) / cellx - 0.5)
            cy_lo = math.ceil((tminy - miny) / celly - 0.5)
            cy_hi = math.floor((tmaxy - miny) / celly - 0.5)
            if cx_lo < 0:
                cx_lo = 0
            if cy_lo < 0:
                cy_lo = 0
            if cx_hi >= grid:
                cx_hi = grid - 1
            if cy_hi >= grid:
                cy_hi = grid - 1
            if cx_lo > cx_hi or cy_lo > cy_hi:
                continue
            # ребра через знаковые площади: точка внутри, если все три одной ориентации
            ay = y1 - y2
            ax = x2 - x1
            by = y2 - y0
            bx = x0 - x2
            s = ay * (x0 - x2) + ax * (y0 - y2)
            if s == 0.0:
                continue
            c0 = -ay * x2 - ax * y2
            c1 = -by * x0 - bx * y0
            for cy in range(cy_lo, cy_hi + 1):
                pyv = py[cy]
                w0_row = ax * pyv + c0
                w1_row = bx * pyv + c1
                base = cy * grid
                for cx in range(cx_lo, cx_hi + 1):
                    pxv = px[cx]
                    w0 = ay * pxv + w0_row
                    w1 = by * pxv + w1_row
                    w2 = s - w0 - w1
                    if (w0 >= 0.0 and w1 >= 0.0 and w2 >= 0.0) or \
                            (w0 <= 0.0 and w1 <= 0.0 and w2 <= 0.0):
                        filled.add(base + cx)
        return filled


def _iou(a, b):
    inter = len(a & b)
    union = len(a | b)
    return inter / union if union else 1.0


def _permute_points(points, perm, signs):
    p0, p1, p2 = perm
    s0, s1, s2 = signs
    return [(s0 * p[p0], s1 * p[p1], s2 * p[p2]) for p in points]


def _transform_points(points, scale, offset):
    ox, oy, oz = offset
    return [(scale * x + ox, scale * y + oy, scale * z + oz) for x, y, z in points]


def best_orientation(ds_tris, ps2_tris, grid=64, allow_mirror=False):
    """Подобрать преобразование PS2 → DS.

    Возвращает dict: perm, signs, scale, offset, mirrored, score, runner_up, ambiguous.
    ds = scale * (signs[j] * ps2[perm[j]]) + offset[j].
    Зеркальные варианты (определитель -1) исключаются, если allow_mirror=False.
    """
    if grid < 1:
        raise ValueError('grid должен быть >= 1')
    ds_tris = _thin(list(ds_tris))
    ps2_tris = _thin(list(ps2_tris))
    if not ds_tris or not ps2_tris:
        raise ValueError('нужны непустые списки треугольников')

    ds_points = [pt for tri in ds_tris for pt in tri]
    ps2_points = [pt for tri in ps2_tris for pt in tri]

    ds_bbox = _bbox(ds_points)
    ds_center = tuple((lo + hi) / 2.0 for lo, hi in ds_bbox)
    ds_extent = [hi - lo for lo, hi in ds_bbox]
    ds_longest = max(ds_extent)
    if ds_longest <= 0.0:
        raise ValueError('DS-модель вырождена')

    # общие границы: bbox DS, расширенный на 10% по каждой стороне
    lo = [ds_bbox[i][0] - 0.1 * ds_extent[i] for i in range(3)]
    hi = [ds_bbox[i][1] + 0.1 * ds_extent[i] for i in range(3)]
    projs = (_Proj(0, 1, lo, hi, grid), _Proj(0, 2, lo, hi, grid), _Proj(1, 2, lo, hi, grid))
    ds_sets = [p.raster(ds_tris) for p in projs]

    results = []
    for perm, signs, mirrored in _CANDIDATES:
        if mirrored and not allow_mirror:
            continue
        permuted = _permute_points(ps2_points, perm, signs)
        ps2_bbox = _bbox(permuted)
        ps2_center = tuple((l + h) / 2.0 for l, h in ps2_bbox)
        ps2_longest = max(h - l for l, h in ps2_bbox)
        if ps2_longest <= 0.0:
            continue
        scale = ds_longest / ps2_longest
        offset = tuple(ds_center[j] - scale * ps2_center[j] for j in range(3))
        flat = _transform_points(permuted, scale, offset)
        tris = list(zip(flat[0::3], flat[1::3], flat[2::3]))
        score = sum(_iou(ds_sets[k], projs[k].raster(tris)) for k in range(3)) / 3.0
        results.append((score, perm, signs, scale, offset, mirrored))

    if not results:
        raise ValueError('PS2-модель вырождена')
    results.sort(key=lambda r: -r[0])
    score, perm, signs, scale, offset, mirrored = results[0]
    runner_up = score
    for r in results[1:]:
        if r[0] < score - 1e-9:
            runner_up = r[0]
            break
    return {
        'perm': perm,
        'signs': signs,
        'scale': scale,
        'offset': offset,
        'mirrored': mirrored,
        'score': score,
        'runner_up': runner_up,
        'ambiguous': score - runner_up < 0.02,
    }


def apply(result, point):
    """Применить найденное преобразование к точке PS2: ds = scale*(signs*ps2[perm]) + offset."""
    perm = result['perm']
    signs = result['signs']
    scale = result['scale']
    offset = result['offset']
    return (scale * signs[0] * point[perm[0]] + offset[0],
            scale * signs[1] * point[perm[1]] + offset[1],
            scale * signs[2] * point[perm[2]] + offset[2])


def read_obj(source):
    """Читает Wavefront OBJ: строки `v` и `f` (формы i, i/j, i/j/k, i//k;
    индексы 1-based и отрицательные). Принимает путь или файлоподобный объект.
    Возвращает список треугольников."""
    if isinstance(source, str):
        with open(source) as f:
            return _read_obj_lines(f)
    return _read_obj_lines(source)


def _read_obj_lines(f):
    vertices = []
    tris = []
    for line in f:
        line = line.strip()
        if not line or line.startswith('#'):
            continue
        parts = line.split()
        if parts[0] == 'v':
            vertices.append((float(parts[1]), float(parts[2]), float(parts[3])))
        elif parts[0] == 'f':
            resolved = []
            for tok in parts[1:]:
                i = int(tok.split('/')[0])
                resolved.append(i - 1 if i > 0 else len(vertices) + i)
            for k in range(1, len(resolved) - 1):
                tris.append((vertices[resolved[0]], vertices[resolved[k]], vertices[resolved[k + 1]]))
    return tris


def main(argv):
    allow_mirror = '--allow-mirror' in argv
    args = [a for a in argv if a != '--allow-mirror']
    if len(args) != 2:
        print('использование: orient.py DS.obj PS2.obj [--allow-mirror]')
        return 2
    ds_tris = read_obj(args[0])
    ps2_tris = read_obj(args[1])
    r = best_orientation(ds_tris, ps2_tris, allow_mirror=allow_mirror)
    print('треугольников: DS %d, PS2 %d' % (len(ds_tris), len(ps2_tris)))
    print('perm=%s signs=%s scale=%.6g offset=(%.6g, %.6g, %.6g)' % (
        r['perm'], r['signs'], r['scale'], r['offset'][0], r['offset'][1], r['offset'][2]))
    print('mirrored=%s score=%.6f runner_up=%.6f ambiguous=%s' % (
        r['mirrored'], r['score'], r['runner_up'], r['ambiguous']))
    print('ds = scale * (signs[j] * ps2[perm[j]]) + offset[j]')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
