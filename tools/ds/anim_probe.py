#!/usr/bin/env python3
"""Current animations of the game and their frames, read from RAM (docs/ULTIMATE-IMPORT.md, У3.6).

    anim_probe.py ROM.nds SAVE.sav SCENE [--run N] [--press KEY] [--frames N] [--model NAME]

Runs `sunpetal-debug` (REPL) to SCENE (Cnn_yy, free walk through --autoplay + skip) and, for N frames, prints
every animation object the game holds: which ROM motion file it plays (matched by content) and its frame.

How the objects look in RAM (NNS_G3d, found at C02_01, see the journal):
  * the game loads each `.nsbca` file raw into a heap block (16-byte header `DU`, size, prev, next) so a file
    in RAM starts with `BCA0`; its `JNT0` block is at +0x14, the animation record (`J\\0AC`) at JNT0 + 0x30
    for the first (only) animation of the file;
  * the animation object is a block with the fields: +0 frame (fx32, 1.0 = 0x1000), +4 ratio (0x1000),
    +8 pointer to the animation record (JNT0 + 0x30 by the first animation), +0xC calc function.
So: find `BCA0` files in RAM, find words equal to the record's address, the object is at word - 8.
"""
import argparse
import hashlib
import json
import os
import struct
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..'))
sys.path.insert(0, HERE)
from ndskit import rom as ndsrom
import nsbca

RAM = 0x02000000
RAM_SIZE = 0x400000
DEBUG = os.path.join(HERE, '..', '..', 'target', 'release', 'sunpetal-debug')


class Repl:
    def __init__(self, rom, save, scene=None, auto=False):
        argv = [DEBUG, rom, '--save', save, '--scale', '1', '--repl']
        if scene and auto:
            argv += ['--goto', scene, '--autoplay']
        self.p = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        if scene and auto:
            self._line()  # the autoplay's own answer
        elif scene:
            self.cmd('goto ' + scene)

    def _line(self):
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise EOFError('sunpetal-debug closed')
            if line.startswith('{'):
                return json.loads(line)

    def cmd(self, c):
        self.p.stdin.write(c + '\n')
        self.p.stdin.flush()
        return self._line()

    def peek(self, addr, n):
        return bytes.fromhex(self.cmd('peek %X:%d' % (addr, n))['hex'])

    def close(self):
        try:
            self.p.stdin.write('quit\n')
            self.p.stdin.flush()
        except OSError:
            pass
        self.p.wait(timeout=10)


def u32(b, o):
    return struct.unpack_from('<I', b, o)[0]


def rom_index(rom):
    """{sha1 of a decompressed .nsbca file: file name}."""
    return {hashlib.sha1(b).hexdigest(): os.path.basename(name) for name, b in nsbca.rom_files(rom)}


def objects(ram, index):
    """[(object address, motion name or '?', frame, record address)] for the animation files in a RAM dump."""
    out = []
    at = 0
    while True:
        at = ram.find(b'BCA0\xff\xfe', at)
        if at < 0:
            break
        size = u32(ram, at + 8)
        f = ram[at:at + size] if 0 < size <= 0x80000 else b''
        if f and len(f) == size and f[0x14:0x18] == b'JNT0':
            name = index.get(hashlib.sha1(f).hexdigest(), '?')
            for k in range(struct.unpack_from('<H', f, 14)[0]):
                rec = RAM + at + u32(f, 16 + 4 * k) + 0x30
                needle = struct.pack('<I', rec)
                p = 0
                while True:
                    p = ram.find(needle, p)
                    if p < 0:
                        break
                    if p % 4 == 0 and p >= 8 and RAM <= u32(ram, p + 4) < RAM + RAM_SIZE:
                        out.append((RAM + p - 8, name, u32(ram, p - 8), rec))
                    p += 4
        at += 1
    return out


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('rom')
    ap.add_argument('save')
    ap.add_argument('scene')
    ap.add_argument('--skip', type=int, default=100, help='frames of skip/advance before the first reading')
    ap.add_argument('--press', default='', help='a button held through the readings (UP, ...)')
    ap.add_argument('--frames', type=int, default=6)
    ap.add_argument('--run', type=int, default=0, help='frames to run before the readings (no --skip)')
    args = ap.parse_args(argv)
    rom = ndsrom.Rom(open(args.rom, 'rb').read())
    index = rom_index(rom)
    r = Repl(args.rom, args.save, args.scene, auto=args.skip > 0)
    try:
        if args.skip > 0:
            r.cmd('skip %d' % args.skip)
            r.cmd('advance 20')
        if args.run:
            r.cmd('run %d' % args.run)
        step = ('press %s 1' % args.press) if args.press else 'advance 1'
        if args.press:
            r.cmd('press %s 10' % args.press)
        for k in range(args.frames):
            ram = r.peek(RAM, RAM_SIZE)
            for addr, name, frame, rec in objects(ram, index):
                print('%d %08X %-14s frame %6.2f  (record %08X)' % (k, addr, name, frame / 4096, rec))
            r.cmd(step)
    finally:
        r.close()
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
