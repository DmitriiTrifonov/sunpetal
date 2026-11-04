#!/usr/bin/env python3
"""Unit tests for frame_states_pack (bbox parsing and corner placement).

No network, no ROM; the tiny synthetic PNGs are drawn with `magick`.
"""
import subprocess, tempfile, unittest

import frame_states_pack as fsp


def magick(*a):
    subprocess.run(['magick', *map(str, a)], check=True)


def make_png(path, w, h, rect=None):
    """A transparent w x h PNG; `rect` is (x0, y0, x1, y1) of an opaque white fill."""
    cmd = ['-size', f'{w}x{h}', 'xc:none']
    if rect:
        x0, y0, x1, y1 = rect
        cmd += ['-fill', 'white', '-draw', f'rectangle {x0},{y0} {x1},{y1}']
    magick(*cmd, path)


class BboxTest(unittest.TestCase):
    def test_bbox_of_rect(self):
        with tempfile.TemporaryDirectory() as d:
            p = d + '/s.png'
            make_png(p, 10, 8, (2, 3, 4, 6))          # 3x4 rect at (2,3)
            self.assertEqual(fsp.bbox_alpha(p), (2, 3, 3, 4))

    def test_bbox_empty(self):
        with tempfile.TemporaryDirectory() as d:
            p = d + '/e.png'
            make_png(p, 5, 5)
            self.assertEqual(fsp.bbox_alpha(p), (0, 0, 0, 0))

    def test_bbox_full(self):
        with tempfile.TemporaryDirectory() as d:
            p = d + '/f.png'
            make_png(p, 6, 4, (0, 0, 5, 3))
            self.assertEqual(fsp.bbox_alpha(p), (0, 0, 6, 4))


class CornerTest(unittest.TestCase):
    SVG = ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 46 62">'
           '<polygon points="a"/><polygon points="b"/>'
           '<polygon points="c"/><polygon points="d"/></svg>')

    def test_corners_order(self):
        tl, bl = fsp.corners(self.SVG)
        self.assertIn('points="a"', tl)
        self.assertIn('points="c"', bl)

    def test_is_bottom(self):
        self.assertFalse(fsp.is_bottom((0, 0, 8, 8), 32))
        self.assertTrue(fsp.is_bottom((0, 24, 8, 8), 32))

    def test_anchor(self):
        self.assertEqual(fsp.anchor((1, 1, 8, 8), False), (4, 4))
        self.assertEqual(fsp.anchor((0, 24, 8, 8), True), (0, 128))


class ComposeTest(unittest.TestCase):
    def test_place_top(self):
        with tempfile.TemporaryDirectory() as d:
            corner = d + '/corner.png'
            make_png(corner, 10, 10, (3, 3, 3, 3))     # tip pixel at (3,3)
            out = d + '/out.png'
            fsp.compose(corner, (1, 1, 8, 8), False, out)
            self.assertEqual(fsp.bbox_alpha(out), (4, 4, 1, 1))

    def test_place_bottom(self):
        with tempfile.TemporaryDirectory() as d:
            corner = d + '/corner.png'
            make_png(corner, 10, 10, (3, 7, 3, 7))     # tip pixel at (3,7)
            out = d + '/out.png'
            fsp.compose(corner, (0, 24, 8, 8), True, out)
            self.assertEqual(fsp.bbox_alpha(out), (0, 127, 1, 1))


if __name__ == '__main__':
    unittest.main()
