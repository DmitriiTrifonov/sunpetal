import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'ps2'))
import compare
import lf


def png(path, w, h, f):
    px = bytearray()
    for y in range(h):
        for x in range(w):
            px += bytes(f(x, y))
    lf.write_png(path, w, h, bytes(px))


class Compare(unittest.TestCase):
    def test_known_difference_and_shift(self):
        with tempfile.TemporaryDirectory() as t:
            a, b, c = (os.path.join(t, n) for n in ('a.png', 'b.png', 'c.png'))
            base = lambda x, y: (x * 4 % 256, y * 4 % 256, 80, 255)
            png(a, 32, 32, base)
            png(b, 32, 32, lambda x, y: (255, 0, 0, 255) if x < 4 and y < 4 else base(x, y))
            png(c, 32, 32, lambda x, y: base(x - 2, y))
            r, img = compare.compare(a, a)
            self.assertTrue(r['same'])
            r, img = compare.compare(a, b)
            self.assertEqual(r['changed'], 16 - sum(1 for x in range(4) for y in range(4) if base(x, y) == (255, 0, 0, 255)))
            self.assertEqual(r['bbox'], [0, 0, 3, 3])
            self.assertEqual(img[0], 96)
            r, _ = compare.compare(a, c)
            self.assertEqual((r['shift']['dx'], r['shift']['dy']), (2, 0))


if __name__ == '__main__':
    unittest.main()
