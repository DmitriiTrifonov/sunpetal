#!/usr/bin/env python3
"""Tests for anim_probe.py without the ROM: python3 tools/ds/test_anim_probe.py"""
import hashlib
import os
import struct
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import anim_probe


def fake_file():
    # BCA0 header (16 bytes, one block), JNT0 at +0x14 with a record at +0x30 from it
    b = bytearray(0x100)
    b[0:6] = b'BCA0\xff\xfe'
    struct.pack_into('<I', b, 8, len(b))
    struct.pack_into('<H', b, 14, 1)
    struct.pack_into('<I', b, 16, 0x14)
    b[0x14:0x18] = b'JNT0'
    return bytes(b)


class Objects(unittest.TestCase):
    def test_finds_the_object_and_its_frame(self):
        f = fake_file()
        ram = bytearray(0x1000)
        ram[0x100:0x100 + len(f)] = f
        rec = anim_probe.RAM + 0x100 + 0x14 + 0x30
        # object at 0x800: frame, ratio, record pointer, calc function
        struct.pack_into('<4I', ram, 0x800, 7 * 4096, 4096, rec, anim_probe.RAM + 0x2000)
        found = anim_probe.objects(bytes(ram) + bytes(0x3000), {hashlib.sha1(f).hexdigest(): 'M0102.nsbca'})
        self.assertEqual(found, [(anim_probe.RAM + 0x800, 'M0102.nsbca', 7 * 4096, rec)])

    def test_unknown_file_is_a_question_mark(self):
        f = fake_file()
        ram = bytearray(0x1000)
        ram[0x100:0x100 + len(f)] = f
        rec = anim_probe.RAM + 0x100 + 0x14 + 0x30
        struct.pack_into('<4I', ram, 0x800, 0, 4096, rec, anim_probe.RAM + 0x2000)
        self.assertEqual(anim_probe.objects(bytes(ram) + bytes(0x3000), {})[0][1], '?')


if __name__ == '__main__':
    unittest.main()
