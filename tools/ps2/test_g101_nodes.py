"""Тесты g101_nodes (K-184) на синтетических данных: без данных игры."""
import os
import struct
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import g101_nodes as gn

IDENT = struct.pack('<16f', 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)


def record(nodes, first=0x170):
    """Запись: пустая шапка и таблица узлов (шаг 0x140): три единичные матрицы, child/sibling, id, смещение и углы."""
    d = bytearray(first + 0x140 * len(nodes) + 0x200)
    for i, (child, sib, t, rot) in enumerate(nodes):
        b = first + 0x140 * i
        for k in range(3):
            d[b + 0x40 + 0x40 * k:b + 0x80 + 0x40 * k] = IDENT
        struct.pack_into('<HH', d, b + 0x104, (child + 1) if child is not None else 0xffff, (sib + 1) if sib is not None else 0xffff)
        struct.pack_into('<I', d, b + 0x10c, 0x77e40000 + i)
        struct.pack_into('<6f', d, b + 0x110, *t, *rot)
    return bytes(d)


def box(x0, y0, z0, x1, y1, z1):
    """Треугольники коробки; вершина — 16 float, как у каталога (x, y, z, 1 | нормаль | цвет | uv)."""
    p = [(x, y, z) for x in (x0, x1) for y in (y0, y1) for z in (z0, z1)]
    quads = [(0, 1, 3, 2), (4, 6, 7, 5), (0, 4, 5, 1), (2, 3, 7, 6), (0, 2, 6, 4), (1, 5, 7, 3)]
    out = []
    for q in quads:
        for tri in ((q[0], q[1], q[2]), (q[0], q[2], q[3])):
            out.append(tuple(p[i] + (1.0, 0, 0, 1, 1.0, 1, 1, 128, 0, 0, 1, 0) for i in tri))
    return out


class Nodes(unittest.TestCase):
    def test_the_table_is_found_at_any_start_and_read(self):
        d = record([(1, None, (0, 0, 0), (0, 0, 0)), (None, None, (6.0, 2.5, 12.0), (0.3, 0, 0))], first=0x170)
        n = gn.read_nodes(d)
        self.assertEqual(len(n), 2)
        self.assertEqual(n[1]['t'], (6.0, 2.5, 12.0))
        self.assertAlmostEqual(n[1]['rot'][0], 0.3, places=5)
        self.assertEqual(len(gn.read_nodes(record([(None, None, (1, 2, 3), (0, 0, 0))], first=0x100))), 1)

    def test_no_table_is_no_nodes(self):
        self.assertEqual(gn.read_nodes(bytes(4096)), [])

    def test_offsets_add_up_along_the_parents(self):
        n = gn.read_nodes(record([(1, None, (1, 0, 0), (0, 0, 0)), (None, None, (0, 2, 0), (0, 0, 0))]))
        self.assertEqual(gn.world_offsets(n)[1], (1.0, 2.0, 0.0))


class Place(unittest.TestCase):
    def car(self):
        body = box(-8, 1, -18, 8, 11, 19)
        wheels = box(-0.3, -2.8, -2.8, 2.0, 2.8, 2.8) + box(-2.0, -2.8, -2.8, 0.3, 2.8, 2.8)
        nodes = gn.read_nodes(record([
            (None, 1, (0, 0, 0), (0, 0, 0)),
            (None, None, (6.3, 2.7, 12.0), (0, 0, 0)),
            (None, None, (-6.3, 2.7, 12.0), (0, 0, 0))]))
        return body + wheels, nodes

    def test_wheels_go_to_their_nodes(self):
        tris, nodes = self.car()
        out, rep = gn.place(tris, nodes)
        self.assertEqual(rep['placed'], 2, rep)
        xs = sorted({round(v[0], 1) for t in out[12:] for v in t})
        self.assertGreater(min(xs), -8.5)
        self.assertLess(max(xs), 8.5)
        # the wheel with the hub outward: x of the right wheel from 6.0 to 8.3
        right = [v for t in out[12:24] for v in t]
        self.assertAlmostEqual(min(v[0] for v in right), 6.0, places=1)
        self.assertAlmostEqual(min(v[1] for v in right), -0.1, places=1)

    def test_a_count_that_does_not_fit_leaves_the_model_alone(self):
        tris, nodes = self.car()
        out, rep = gn.place(tris, nodes[:2])
        self.assertEqual(rep['placed'], 0)
        self.assertEqual(out, tris)
        self.assertIn('local parts 2', rep['reason'])

    def test_without_nodes_nothing_changes(self):
        tris, _ = self.car()
        self.assertEqual(gn.place(tris, [])[0], tris)

    def test_rotation_turns_the_part(self):
        r = gn._rot((0.0, 0.0, 1.5707963))
        p = gn._apply((1.0, 0.0, 0.0, 1.0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0), r, (0, 0, 0))
        self.assertAlmostEqual(p[0], 0.0, places=5)
        self.assertAlmostEqual(p[1], 1.0, places=5)


if __name__ == '__main__':
    unittest.main()
