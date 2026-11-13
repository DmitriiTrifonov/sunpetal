#!/usr/bin/env python3
"""Tests for nsbca.py without the ROM: python3 tools/ds/test_nsbca.py"""
import os
import struct
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import nsbca
import nsbmd


class Decode(unittest.TestCase):
    def test_pivot_entry_matches_the_model_pivot(self):
        # M0102's first table entry: cell 1, no negation = the pelvis rest matrix of the hero
        m = nsbca.pivot_entry(1, 0, -4096)
        self.assertEqual(m[0], [0.0, 1.0, 0.0])
        self.assertEqual(m[1], [0.0, 0.0, -1.0])
        self.assertEqual(m[2], [-1.0, 0.0, 0.0])

    def test_pivot_negation_bits(self):
        # info 36 = cell 4 (the middle one), bit 0x20: a rotation about Y
        a, b = 3705, 1747
        m = nsbca.pivot_entry(36, a, b)
        self.assertEqual(m[1], [0.0, 1.0, 0.0])
        self.assertAlmostEqual(m[0][0], a / 4096)
        self.assertAlmostEqual(m[0][2], b / 4096)
        self.assertAlmostEqual(m[2][0], -b / 4096)

    def test_rot5_is_a_rotation(self):
        m = nsbca.rot5_entry(-32401, 44, 4836, -301, -32719)
        for p in range(3):
            for q in range(3):
                dot = sum(m[p][k] * m[q][k] for k in range(3))
                self.assertAlmostEqual(dot, 1.0 if p == q else 0.0, delta=0.01)

    def test_rot5_sign_bit_of_e(self):
        pos = nsbca.rot5_entry(32767, 0, 0, 0, 32766)
        neg = nsbca.rot5_entry(32767, 0, 0, 0, 32767)
        self.assertGreaterEqual(pos[1][2], 0)
        self.assertLessEqual(neg[1][2], 0)

    def test_tracks_layout(self):
        # translation x const, y animated, z const; rotation animated; scale identity (bit 9)
        flags = 1 << 3 | 1 << 5 | 1 << 9
        words = [7, 0x401C0000, 0x100, 9, 0x401C0000, 0x200]
        t, r = nsbca.tracks(flags, words)
        self.assertEqual(t[0], ('const', 7))
        self.assertEqual(t[1], ('anim', 0x401C0000, 0x100))
        self.assertEqual(t[2], ('const', 9))
        self.assertEqual(r, ('anim', 0x401C0000, 0x200))

    def test_tracks_identity_translation_and_const_rotation(self):
        t, r = nsbca.tracks(1 << 1 | 1 << 3 | 1 << 4 | 1 << 5 | 1 << 8, [0x8000])
        self.assertEqual(t, [('const', 0)] * 3)
        self.assertEqual(r, ('const', 0x8000))

    def test_sample_shape_and_count(self):
        self.assertEqual(nsbca.sample_shape(0x601C0000), (1, True, 28))
        self.assertEqual(nsbca.sample_shape(0x81F00000), (2, False, 496))
        # 30 frames, every second: 15 steps + the last odd frame -> 16 samples (M0102's pool stride)
        self.assertEqual(nsbca.sample_count(30, 0x401C0000), 16)
        self.assertEqual(nsbca.sample_count(31, 0x601E0000), 16)
        self.assertEqual(nsbca.sample_count(60, 0x003C0000), 60)

    def test_pose_reads_samples(self):
        # one node, translation x animated s32 every 2 frames, y and z const, rotation identity
        pool = struct.pack('<4i', 4096, 8192, 12288, 16384)
        buf = bytes(8) + pool
        anim = {'nodes': [{'flags': 1 << 4 | 1 << 5 | 1 << 6 | 1 << 8 | 1 << 9,
                           'words': [0x40040000, 8, 0, 0]}],  # x pair, y, z const
                'tables': (0, 0)}
        t, r = nsbca.pose(buf, anim, 0, 2, 0)
        self.assertAlmostEqual(t[0], 2.0)
        self.assertEqual(r, [[1.0, 0, 0], [0, 1.0, 0], [0, 0, 1.0]])
        t, _ = nsbca.pose(buf, anim, 0, 4, 0)
        self.assertAlmostEqual(t[0], 3.0)


class WorldMatrices(unittest.TestCase):
    def test_chain_of_two_nodes(self):
        class M:  # two nodes, the second under the first
            nodes = [{'s': (1.0, 1.0, 1.0), 'parent': None, 'local': None},
                     {'s': (1.0, 1.0, 1.0), 'parent': 0, 'local': None}]
        # both nodes: translation identity (bit 1 + 3,4,5), rotation identity (6, 8), scale identity: no words
        # but the first one translates x by 2 (const) and the second y by 3
        flags = 1 << 6 | 1 << 8 | 1 << 9 | 1 << 4 | 1 << 5
        anim = {'at': 0, 'tables': (0, 0), 'nodes': [
            {'flags': flags | 1 << 3, 'words': [2 * 4096, 0, 0]},
            {'flags': flags | 1 << 3, 'words': [0, 3 * 4096, 0]}]}
        w = nsbca.world_matrices(M, b'', anim, 0)
        self.assertEqual(w[0][3], [2.0, 0.0, 0.0])
        self.assertEqual(w[1][3], [2.0, 3.0, 0.0])


if __name__ == '__main__':
    unittest.main()
