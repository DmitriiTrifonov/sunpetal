#!/usr/bin/env python3
"""Тесты golden.py без игры: python3 tools/auto/test_golden.py"""
import os
import shutil
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, '..', 'ps2'))
import golden
import lf


def frame(path, w=8, h=4, rgb=(10, 20, 30)):
    lf.write_png(path, w, h, bytes(rgb + (255,)) * (w * h))


class Compare(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp()
        self.base, self.out = os.path.join(self.d, 'base'), os.path.join(self.d, 'out')
        os.makedirs(self.base)
        os.makedirs(self.out)

    def tearDown(self):
        shutil.rmtree(self.d)

    def both(self, name, new_rgb=(10, 20, 30)):
        frame(os.path.join(self.base, name + '.png'))
        frame(os.path.join(self.out, name + '.png'), rgb=new_rgb)

    def test_equal_frames_are_the_same(self):
        self.both('a')
        self.assertEqual([(n, s) for n, s, _ in golden.compare(self.out, self.base, ['a'])], [('a', 'same')])

    def test_a_colour_change_is_caught_and_measured(self):
        self.both('a', (10, 20, 40))   # a blue channel 10 higher everywhere
        (_, st, d), = golden.compare(self.out, self.base, ['a'])
        self.assertEqual(st, 'changed')
        self.assertEqual((d['changed'], d['max'], d['bbox']), (32, 10, (0, 0, 7, 3)))

    def test_one_pixel(self):
        self.both('a')
        w, h, px = lf.read_png(os.path.join(self.out, 'a.png'))
        px = bytearray(px)
        px[(2 * w + 5) * 4] += 1
        lf.write_png(os.path.join(self.out, 'a.png'), w, h, bytes(px))
        (_, st, d), = golden.compare(self.out, self.base, ['a'])
        self.assertEqual((st, d['changed'], d['bbox']), ('changed', 1, (5, 2, 5, 2)))

    def test_missing_and_new(self):
        frame(os.path.join(self.out, 'n.png'))
        frame(os.path.join(self.base, 'm.png'))
        rows = golden.compare(self.out, self.base, ['n', 'm'])
        self.assertEqual([(n, s) for n, s, _ in rows], [('n', 'new'), ('m', 'missing')])

    def test_size_change(self):
        frame(os.path.join(self.base, 'a.png'))
        frame(os.path.join(self.out, 'a.png'), w=9)
        (_, st, d), = golden.compare(self.out, self.base, ['a'])
        self.assertEqual((st, d['size']), ('changed', ((8, 4), (9, 4))))

    def test_report_names_the_change_and_keeps_the_pairs(self):
        self.both('a', (200, 20, 30))
        self.both('b')
        rows = golden.compare(self.out, self.base, ['a', 'b'])
        md = open(golden.report(self.out, self.base, rows), encoding='utf-8').read()
        self.assertIn('**a**: изменился', md)
        self.assertNotIn('**b**', md)
        for suffix in ('_base', '_new', '_diff'):
            self.assertTrue(os.path.isfile(os.path.join(self.out, 'report', 'a' + suffix + '.png')))

    def test_places_file_is_sound(self):
        places = golden.load_places()
        self.assertGreaterEqual(len(places), 18)
        for p in places:
            self.assertRegex(p['goto'], r'^C\d\d_\d\d$')


if __name__ == '__main__':
    unittest.main()
