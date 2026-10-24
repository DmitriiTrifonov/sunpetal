"""The keys nitro.py works out must be the ones texpack.rs works out (golden values shared with the
Rust test `keys_agree_with_the_tools_python_side`).   python3 tools/ds/test_nitro.py"""
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, '..'))
import nitro


def tex(fmt, c0, data, info=None):
    return nitro.Texture('t', fmt, 8, 8, c0, bytes(data), info)


class Keys(unittest.TestCase):
    PAL = bytes((i * 7) % 256 for i in range(512))

    def test_golden(self):
        info = b''.join(v.to_bytes(2, 'little') for v in (0x0004, 0x4008, 0x8002, 0xC00A))
        cases = [
            (tex(4, 0, [(i * 3) % 251 for i in range(64)]), 0xa7c9424195909dfb, 0xfffe2d7271ab9e95),
            (tex(4, 1, [(i * 3) % 251 for i in range(64)]), 0x06e0d4a067869b3c, 0xfffe2d7271ab9e95),
            (tex(2, 1, [(i * 5) % 256 for i in range(16)]), 0x435f037b0f9d16c2, 0x4f9838d6bdc8a675),
            (tex(3, 0, [(i * 11) % 256 for i in range(32)]), 0xc955aa84ddc8781c, 0xb0b2c391709c62a5),
            (tex(5, 0, [(i * 13) % 256 for i in range(16)], info), 0x65f913cef1b9d6e2, 0xb972c4060ba7c695),
        ]
        for t, data, pal in cases:
            k = nitro.key(t, self.PAL)
            self.assertEqual((k[0], k[1]), (data, pal), f'format {t.fmt}')

    def test_a_palette_colour_past_the_used_range_is_not_part_of_the_key(self):
        t = tex(4, 0, [i % 8 for i in range(64)])
        a = nitro.key(t, bytes(512))
        changed = bytearray(512)
        changed[100] = 9
        self.assertEqual(a, nitro.key(t, bytes(changed)))
        changed[4] = 1
        self.assertNotEqual(a, nitro.key(t, bytes(changed)))


if __name__ == '__main__':
    unittest.main()
