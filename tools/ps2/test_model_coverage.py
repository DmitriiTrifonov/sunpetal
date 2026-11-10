"""Тесты model_coverage (K-112) на синтетических таблицах: без данных игры."""
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import model_coverage as mc


def write(path, text):
    with open(path, 'w') as f:
        f.write(text)


class Coverage(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.TemporaryDirectory()
        p = os.path.join(self.d.name, 'pairs.csv')
        write(p, 'name,source,mae\n'
                 '64x64_5_aa_bb.png,DATA/EVENT/R01.BIN#1.2,1\n'
                 '64x64_5_cc_dd.png,DATA/EVENT/R01.BIN#2.0,1\n'
                 '32x32_5_ee_ff.png,DATA/EVENT/R01.BIN#9.0,1\n'
                 '16x16_5_11_22.png,,1\n')
        c = os.path.join(self.d.name, 'cat.csv')
        write(c, 'архив,запись,вершин,треугольников,картинки,m_stacked,m_parts,m_status\n'
                 'R01,1,10,5,001_2.png:64x64,0,0,ok\n'
                 'R01,2,20,9,002_0.png:64x64 002_2.png:32x32,6,0,bad\n'
                 'R01,3,5,2,,0,0,ok\n')
        self.pairs, self.keys = mc.read_pairs(p)
        self.models = mc.read_models([c])

    def tearDown(self):
        self.d.cleanup()

    def test_pairs(self):
        self.assertEqual(len(self.pairs), 3)
        self.assertEqual(len(self.keys), 4)
        self.assertEqual(self.pairs[0], ('64x64_5_aa_bb', 'R01', 1, 2))

    def test_decisions(self):
        rows = {r['запись']: r for r in mc.build(self.pairs, self.models)}
        self.assertEqual(rows[1]['решение'], 'swap')
        self.assertEqual(rows[2]['решение'], 'blocked')
        self.assertEqual(rows[2]['класс'], 'составная')
        self.assertEqual(rows[3]['решение'], 'unpaired')

    def test_ds_only_and_nomesh(self):
        self.assertEqual(mc.ds_only(self.keys, self.pairs), ['16x16_5_11_22'])
        self.assertEqual(mc.textures_without_mesh(self.pairs, self.models), [('32x32_5_ee_ff', 'R01', 9)])


if __name__ == '__main__':
    unittest.main()
