"""nsbmd.py on a BMD0 file built here (no ROM): one model, one node, one material, one triangle.
python3 tools/ds/test_nsbmd.py   (with SUNPETAL_ROM=file.nds: also every model file of the ROM)"""
import os
import struct
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, '..'))
import nsbmd


def dictionary(entries, unit):
    """A NITRO dictionary: [(name, entry bytes)]."""
    n = len(entries)
    head = struct.pack('<BBHHH', 0, n, 8 + 4 * (n + 1) + 4 + unit * n + 16 * n, 8, 8 + 4 * (n + 1))
    return (head + bytes(4 * (n + 1)) + struct.pack('<HH', unit, 4 + unit * n)
            + b''.join(e for _, e in entries) + b''.join(nm.encode().ljust(16, b'\0') for nm, _ in entries))


def build(rot_flags=6, node=None, pos_scale=1 << 12):
    w = lambda *v: struct.pack('<%dI' % len(v), *v)
    node = node if node is not None else struct.pack('<HhIII', rot_flags, 0, 4096, 0, 0)[:16]
    # display list: BEGIN_VTXS(triangles), three VTX_16, END
    # explicit words: v0 = (4096,0,0), v1 = (0,4096,0), v2 = (0,0,4096) as (x | y << 16, z)
    dl = bytes([0x40, 0x23, 0x23, 0x23]) + w(0) + w(4096, 0) + w(4096 << 16, 0) + w(0, 4096) + bytes([0x41, 0, 0, 0])
    node_dict_len = len(dictionary([('root', bytes(4))], 4))
    node_at = 0x40 + node_dict_len                      # relative to the model
    sbc = bytes([0x06, 0, 0, 0, 0x02, 0, 1, 0x04, 0, 0x05, 0, 0x01])
    sbc_at = node_at + len(node)
    sbc_at += -sbc_at % 4
    mat_at = sbc_at + len(sbc)
    mat_at += -mat_at % 4
    mat_dict_len = len(dictionary([('mat', bytes(4))], 4))
    item_at = 4 + mat_dict_len                          # relative to the materials block
    item = bytearray(0x2C)
    struct.pack_into('<I', item, 12, 0x1F00C0)          # polygon attributes
    shp_at = mat_at + item_at + len(item)
    shp_dict_len = len(dictionary([('shp', bytes(4))], 4))
    shape_at = shp_dict_len                             # relative to the shapes dictionary
    shape = struct.pack('<HHIII', 0, 16, 0, 16, len(dl))
    model_size = shp_at + shape_at + len(shape) + len(dl)
    head = struct.pack('<IIIII', model_size, sbc_at, mat_at, shp_at, 0)
    head += bytes([0, 0, 0, 1, 1, 1, 0, 0]) + struct.pack('<ii', pos_scale, 4096 * 4096 // pos_scale)
    head += struct.pack('<4H', 3, 1, 1, 0) + struct.pack('<6h', 0, 0, 0, 4096, 4096, 4096) + struct.pack('<ii', 4096, 4096)
    assert len(head) == 0x40
    m = head + dictionary([('root', struct.pack('<I', node_at - 0x40))], 4)
    m += node
    m += bytes(sbc_at - len(m)) + sbc
    m += bytes(mat_at - len(m))
    m += struct.pack('<HH', 0, 0) + dictionary([('mat', struct.pack('<I', item_at))], 4)
    m += bytes(item_at - (len(m) - mat_at)) + bytes(item)
    m += dictionary([('shp', struct.pack('<I', shape_at))], 4) + shape + dl
    assert len(m) == model_size, (len(m), model_size)
    mdict = dictionary([('M', struct.pack('<I', 0))], 4)
    mdl_off = 8 + len(mdict)
    mdict = dictionary([('M', struct.pack('<I', mdl_off))], 4)
    blk = b'MDL0' + struct.pack('<I', 8 + len(mdict) + len(m)) + mdict + m
    return b'BMD0' + struct.pack('<HHIHHI', 0xFEFF, 1, 16 + len(blk), 16, 1, 20) + blk


class Synthetic(unittest.TestCase):
    def test_model(self):
        b = build()
        (m,) = nsbmd.parse(b)
        self.assertEqual(m.name, 'M')
        self.assertEqual([n['name'] for n in m.nodes], ['root'])
        self.assertEqual(m.nodes[0]['t'], (1.0, 0.0, 0.0))
        self.assertEqual(m.nodes[0]['world'][3], [1.0, 0.0, 0.0])
        self.assertEqual(m.materials[0]['poly_attr'], 0x1F00C0)
        self.assertEqual(m.draws, [{'node': 0, 'mat': 0, 'shape': 0, 'restore': None}])
        (_, tri), = nsbmd.triangles(b, *m.shapes[0]['dl'])
        self.assertEqual(tri, [(1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0)])

    def test_vtx_diff_adds_the_bits_as_they_are(self):
        # BEGIN_VTXS(triangles), VTX_16 (4096, 0, 0), VTX_DIFF (+256, -256, 0), VTX_DIFF (0, 0, +511), END
        w = lambda *v: struct.pack('<%dI' % len(v), *v)
        dl = bytes([0x40, 0x23, 0x28, 0x28]) + w(0) + w(4096, 0) + w(256 | (1024 - 256) << 10) + w(511 << 20) + bytes([0x41, 0, 0, 0])
        (_, tri), = nsbmd.triangles(dl, 0, len(dl))
        self.assertEqual(tri, [(1.0, 0.0, 0.0), (1.0625, -0.0625, 0.0), (1.0625, -0.0625, 511 / 4096)])

    def test_pivot_rotation_is_a_rotation(self):
        for p in range(9):
            for sign in (0x200, 0x400, 0x300, 0x500):      # [[A B] [-B A]] and [[A B] [B -A]], one cell +-1
                m = nsbmd.pivot_rotation(8 | p << 4 | sign, 0.6, 0.8)
                for i in range(3):
                    for j in range(3):
                        self.assertAlmostEqual(sum(m[i][k] * m[j][k] for k in range(3)), float(i == j))

    def test_truncated_file_is_an_error_not_a_crash(self):
        b = build()
        for cut in (20, 60, len(b) - 10):
            with self.assertRaises((nsbmd.NsbmdError, struct.error, IndexError)):
                nsbmd.parse(b[:cut])

    def test_unknown_sbc_command(self):
        with self.assertRaises(nsbmd.NsbmdError):
            nsbmd.sbc_args(0x1E, b'', 0)


@unittest.skipUnless(os.environ.get('SUNPETAL_ROM'), 'SUNPETAL_ROM not set')
class Rom(unittest.TestCase):
    def test_every_file(self):
        from ndskit import rom as ndsrom
        rom = ndsrom.Rom(open(os.environ['SUNPETAL_ROM'], 'rb').read())
        n = 0
        for path, b in nsbmd.rom_models(rom):
            for m in nsbmd.parse(b):
                t = sum(len(nsbmd.triangles(b, *m.shapes[d['shape']]['dl'])) for d in m.draws)
                self.assertEqual(t, m.n_tri + 2 * m.n_quad, path)
                for nd in m.nodes:
                    r = nd['r']
                    for i in range(3):
                        for j in range(3):
                            self.assertAlmostEqual(sum(r[i][k] * r[j][k] for k in range(3)), float(i == j), delta=0.01, msg=path)
            n += 1
        self.assertGreaterEqual(n, 259)


if __name__ == '__main__':
    unittest.main()
