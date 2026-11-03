import json, os, sys, tempfile, unittest
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import swapman

TRI = [[(1.0, 2.0, 3.0) + (0,) * 9 + (0.25, 0.75)] * 1 + [(4.0, 5.0, 6.0) + (0,) * 9 + (0.5, 0.5)] + [(7.0, 8.0, 9.0) + (0,) * 9 + (0.0, 1.0)]]
SW = {'key': '8x8_5_00000000000000aa_00000000000000bb', 'size': 8, 'hide': ['4x4_5_00000000000000cc_00000000000000dd'],
      'ps2': {'archive': 'X.BIN', 'entry': 3}, 'orient': {'perm': [0, 2, 1], 'signs': [-1, 1, 1], 'scale': 0.5, 'offset': [1, 2, 3]}, 'color': 90}


class T(unittest.TestCase):
    def setUp(self):
        swapman.mesh = lambda raw, a, e: [[tuple(r) for r in t] for t in TRI]

    def test_matrix_and_flipfront(self):
        E, off = swapman.matrix({'perm': [0, 2, 1], 'signs': [-1, 1, 1], 'scale': 2})
        self.assertEqual(E, [[-2, 0, 0], [0, 0, 2], [0, 2, 0]])
        E2, _ = swapman.matrix({'perm': [0, 2, 1], 'signs': [-1, 1, 1], 'scale': 2, 'flipfront': True})
        self.assertEqual((E2[0][0], E2[2][1]), (2, -2))
        self.assertEqual(off, [0, 0, 0])

    def test_build_lines(self):
        t = swapman.build(SW, 'raw').splitlines()
        self.assertEqual(t[:3], ['key ' + SW['key'], 'size 8', 'hide ' + SW['hide'][0]])
        self.assertTrue(t[3].startswith('e -0.500000 0.000000 0.000000'))
        self.assertTrue(t[3].endswith(' 1.000000 2.000000 3.000000'))
        self.assertEqual(t[4], 'v 1.0000 2.0000 3.0000 0.25000 0.75000 90 90 90')
        self.assertEqual(len([x for x in t if x.startswith('v ')]), 3)
        sw = dict(SW, rev=True, vflip=True)
        v = [x for x in swapman.build(sw, 'raw').splitlines() if x.startswith('v ')]
        self.assertEqual(v[1].split()[1:4], ['7.0000', '8.0000', '9.0000'])
        self.assertEqual(v[0].split()[5], '0.25000')   # v = 1 - 0.75

    def test_pick_and_run_write_atomically(self):
        d = tempfile.mkdtemp()
        man = os.path.join(d, 'm.json')
        json.dump({'swaps': {'a': SW, 'b': SW}}, open(man, 'w'))
        with self.assertRaises(SystemExit):
            swapman.run(man, os.path.join(d, 'o.swap'))
        with self.assertRaises(SystemExit):
            swapman.run(man, os.path.join(d, 'o.swap'), 'zzz')
        self.assertEqual(swapman.run(man, os.path.join(d, 'o.swap'), 'a'), 3)
        self.assertFalse(os.path.exists(os.path.join(d, 'o.swap.tmp')))


unittest.main() if __name__ == '__main__' else None
