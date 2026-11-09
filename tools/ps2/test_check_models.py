"""Тесты check_models (K-183) на синтетических OBJ: без данных игры."""
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import check_models as c


def write_obj(path, boxes):
    """По коробке (x0,y0,z0,x1,y1,z1) — 12 треугольников; вершины у каждого треугольника свои, как в каталоге."""
    pts, faces = [], []
    for (x0, y0, z0, x1, y1, z1) in boxes:
        p = [(x, y, z) for x in (x0, x1) for y in (y0, y1) for z in (z0, z1)]
        quads = [(0, 1, 3, 2), (4, 6, 7, 5), (0, 4, 5, 1), (2, 3, 7, 6), (0, 2, 6, 4), (1, 5, 7, 3)]
        for q in quads:
            for tri in ((q[0], q[1], q[2]), (q[0], q[2], q[3])):
                faces.append(tuple(len(pts) + k for k in range(3)))
                pts.extend(p[i] for i in tri)
    with open(path, "w") as f:
        for p in pts:
            f.write("v %f %f %f\n" % p)
        for t in faces:
            f.write("f %d %d %d\n" % tuple(i + 1 for i in t))


class Checks(unittest.TestCase):
    def metrics(self, boxes):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "m.obj")
            write_obj(p, boxes)
            m = c.metrics(p)
        m["status"], m["reason"] = c.verdict(m)
        return m

    def test_one_solid_part_is_fine(self):
        m = self.metrics([(0, 0, 0, 10, 4, 20)])
        self.assertEqual((m["comps"], m["stacked"], m["status"]), (1, 0, "ok"))
        self.assertEqual(m["integrity"], 1.0)

    def test_a_body_with_symmetric_wheels_is_fine(self):
        boxes = [(-5, 2, -10, 5, 6, 10)] + [(x, 0, z, x + 1, 2, z + 2) for x in (-6, 5) for z in (-8, 6)]
        m = self.metrics(boxes)
        self.assertEqual(m["status"], "ok", m["reason"])
        self.assertGreaterEqual(m["sym_x"], 0.95)

    def test_parts_piled_in_the_middle_are_bad(self):
        # eight parts of the same size in one place: doors and wheels that were never moved
        boxes = [(-5, 2, -10, 5, 6, 10)] + [(-1 + 0.05 * i, 0, -1, 1 + 0.05 * i, 2, 1) for i in range(8)]
        m = self.metrics(boxes)
        self.assertEqual(m["status"], "bad")
        self.assertIn("stacked", m["reason"])

    def test_loose_triangles_are_shards(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "s.obj")
            with open(p, "w") as f:
                for i in range(6):
                    for v in ((i * 10, 0, 0), (i * 10 + 1, 0, 0), (i * 10, 1, 0)):
                        f.write("v %f %f %f\n" % v)
                for i in range(6):
                    f.write("f %d %d %d\n" % (3 * i + 1, 3 * i + 2, 3 * i + 3))
            m = c.metrics(p)
        self.assertEqual(c.verdict(m)[0], "bad")
        self.assertIn("shards", c.verdict(m)[1])

    def test_a_broken_index_is_not_integrity(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "b.obj")
            with open(p, "w") as f:
                f.write("v 0 0 0\nv 1 0 0\nv nan 1 0\nf 1 2 3\n")
            m = c.metrics(p)
        self.assertEqual(c.verdict(m)[0], "bad")

    def test_the_reference_lists_do_not_overlap(self):
        self.assertFalse(set(c.REFERENCE_OK) & set(c.REFERENCE_BAD))


if __name__ == "__main__":
    unittest.main()
