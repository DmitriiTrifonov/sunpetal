import os, struct, sys, unittest
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import swapman, mkmodels


def row(x, y, z, u, v):
    return (x, y, z, 0, 0.0, 0.0, 1.0) + (0,) * 5 + (u, v)


SW = {'key': '8x8_5_00000000000000aa_00000000000000bb', 'ps2': {'archive': 'X', 'entry': 1},
      'orient': {'perm': [0, 2, 1], 'signs': [1, 1, 1], 'scale': 2.0, 'offset': [1, 0, 0]}}


class T(unittest.TestCase):
    def setUp(self):
        a, b, c = row(0, 0, 0, 0, 0), row(1, 0, 0, 1, 0), row(0, 1, 0, 0, 1)
        swapman.mesh = lambda raw, ar, e: [[a, b, c], [c, b, a], [(float('nan'),) * 14] * 3]

    def test_shared_vertices_are_stored_once_and_orientation_is_baked(self):
        verts, idx = mkmodels.convert(SW, 'raw')
        self.assertEqual(len(verts), 3)
        self.assertEqual(idx, [0, 1, 2, 2, 1, 0])
        self.assertEqual(verts[1][:3], (3.0, 0.0, 0.0))    # x*2 + 1
        self.assertEqual(verts[2][:3], (1.0, 0.0, 2.0))    # y goes to z by perm (0,2,1)
        self.assertAlmostEqual(verts[0][3:6][1], 1.0)      # normal z → y, unit length, no shift
        rv, ri = mkmodels.convert(dict(SW, rev=True), 'raw')
        self.assertEqual(rv[1][:3], verts[2][:3])    # the second vertex met is the third one: the winding is reversed

    def test_encoding_matches_the_rust_layout(self):
        verts, idx = mkmodels.convert(SW, 'raw')
        b = mkmodels.encode(mkmodels.parse_key(SW['key']), verts, idx)
        self.assertEqual(b[:5], b'SPM1\x01')
        self.assertEqual(struct.unpack('<QQBHH', b[5:26]), (0xaa, 0xbb, 5, 8, 8))
        self.assertEqual(struct.unpack('<II', b[26:34]), (3, 6))
        self.assertEqual(len(b), 34 + 3 * 32 + 6 * 4)
        self.assertEqual(mkmodels.encode(None, verts, idx)[:5], b'SPM1\x00')


unittest.main() if __name__ == '__main__' else None
