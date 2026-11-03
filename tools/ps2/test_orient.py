"""Тесты K-127 orient.py.  python3 -m unittest tools/ps2/test_orient.py

Всё синтетическое, файлов игры нет.
"""
import io
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import orient


def box(lo, hi):
    """12 треугольников прямоугольной призмы от lo до hi (оси выровнены)."""
    x0, y0, z0 = lo
    x1, y1, z1 = hi
    v = [
        (x0, y0, z0), (x1, y0, z0), (x1, y1, z0), (x0, y1, z0),
        (x0, y0, z1), (x1, y0, z1), (x1, y1, z1), (x0, y1, z1),
    ]
    quads = [(0, 1, 2, 3), (4, 5, 6, 7), (0, 1, 5, 4),
             (2, 3, 7, 6), (0, 3, 7, 4), (1, 2, 6, 5)]
    tris = []
    for q in quads:
        tris.append((v[q[0]], v[q[1]], v[q[2]]))
        tris.append((v[q[0]], v[q[2]], v[q[3]]))
    return tris


def car():
    """Асимметричная «машина» из призм разного размера (без симметрии по осям)."""
    tris = []
    tris += box((0.0, 0.0, 0.0), (40.0, 18.0, 12.0))   # кузов
    tris += box((8.0, 5.0, 12.0), (22.0, 13.0, 22.0))  # салон
    tris += box((0.0, 0.0, 12.0), (6.0, 18.0, 14.0))   # передний бампер
    tris += box((36.0, 2.0, 12.0), (40.0, 16.0, 18.0)) # задний спойлер
    tris += box((14.0, 0.0, 14.0), (18.0, 2.0, 20.0))  # зеркало (ломает симметрию по y)
    return tris


def cube():
    return box((-1.0, -1.0, -1.0), (1.0, 1.0, 1.0))


def ds_to_ps2(point, perm, signs, scale, offset):
    """Обратное построение PS2-точки: ds = scale*(signs*ps2[perm]) + offset."""
    ps2 = [0.0, 0.0, 0.0]
    for j in range(3):
        k = perm[j]
        ps2[k] = signs[j] * (point[j] - offset[j]) / scale
    return tuple(ps2)


def to_ps2_tris(ds_tris, perm, signs, scale, offset):
    return [tuple(ds_to_ps2(p, perm, signs, scale, offset) for p in tri) for tri in ds_tris]


# Шесть+ неотражённых преобразований (определитель +1), известные заранее.
TRANSFORMS = [
    ((0, 1, 2), (1, 1, 1), 10.0, (100.0, -50.0, 25.0)),
    ((1, 2, 0), (1, 1, 1), 10.0, (-30.0, 70.0, 5.0)),
    ((2, 0, 1), (1, 1, 1), 10.0, (40.0, 10.0, -60.0)),
    ((0, 1, 2), (-1, -1, 1), 10.0, (0.0, 0.0, 0.0)),
    ((1, 2, 0), (-1, 1, -1), 10.0, (12.0, -8.0, 33.0)),
    ((0, 2, 1), (-1, 1, 1), 10.0, (-15.0, 22.0, -4.0)),
    ((1, 0, 2), (-1, 1, 1), 10.0, (55.0, -40.0, 18.0)),
]


class BestOrientation(unittest.TestCase):
    def test_recovers_known_transforms(self):
        ds_tris = car()
        for perm, signs, scale, offset in TRANSFORMS:
            ps2_tris = to_ps2_tris(ds_tris, perm, signs, scale, offset)
            r = orient.best_orientation(ds_tris, ps2_tris)
            self.assertEqual(r['perm'], perm, (perm, signs))
            self.assertEqual(r['signs'], signs, (perm, signs))
            self.assertAlmostEqual(r['scale'], scale, places=6)
            for j in range(3):
                self.assertAlmostEqual(r['offset'][j], offset[j], places=4)
            self.assertFalse(r['mirrored'])
            self.assertGreater(r['score'], 0.99)
            self.assertFalse(r['ambiguous'])

    def test_apply_maps_ps2_back_to_ds(self):
        ds_tris = car()
        perm, signs, scale, offset = TRANSFORMS[4]
        ps2_tris = to_ps2_tris(ds_tris, perm, signs, scale, offset)
        r = orient.best_orientation(ds_tris, ps2_tris)
        for pt in [(0.0, 0.0, 0.0), (40.0, 0.0, 0.0), (22.0, 13.0, 22.0), (17.0, 1.0, 20.0)]:
            back = orient.apply(r, ds_to_ps2(pt, perm, signs, scale, offset))
            for j in range(3):
                self.assertAlmostEqual(back[j], pt[j], places=3)

    def test_mirror_not_chosen_without_allow_mirror(self):
        ds_tris = car()
        perm, signs, scale, offset = (0, 1, 2), (1, 1, -1), 10.0, (5.0, 5.0, 5.0)
        ps2_tris = to_ps2_tris(ds_tris, perm, signs, scale, offset)
        r = orient.best_orientation(ds_tris, ps2_tris)
        self.assertFalse(r['mirrored'])
        self.assertLess(r['score'], 0.99)
        # с разрешением отражения — восстанавливается зеркало
        rm = orient.best_orientation(ds_tris, ps2_tris, allow_mirror=True)
        self.assertTrue(rm['mirrored'])
        self.assertEqual(rm['perm'], perm)
        self.assertEqual(rm['signs'], signs)
        self.assertGreater(rm['score'], 0.99)

    def test_symmetric_cube_is_ambiguous(self):
        ds_tris = cube()
        r = orient.best_orientation(ds_tris, cube())
        self.assertGreater(r['score'], 0.99)
        self.assertTrue(r['ambiguous'])


class ObjParsing(unittest.TestCase):
    def test_negative_indices_and_slash_forms(self):
        src = io.StringIO(
            'v 0 0 0\n'
            'v 1 0 0\n'
            'v 0 1 0\n'
            'v 0 0 1\n'
            'f 1 2 3\n'
            'f -1 -2 -3\n'
            'f 1/1/1 2/2/2 4/3/3\n'
            'f 2//1 3//2 4//3\n'
        )
        tris = orient.read_obj(src)
        v = [(0.0, 0.0, 0.0), (1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0)]
        self.assertEqual(tris, [
            (v[0], v[1], v[2]),
            (v[3], v[2], v[1]),
            (v[0], v[1], v[3]),
            (v[1], v[2], v[3]),
        ])


if __name__ == '__main__':
    unittest.main()
