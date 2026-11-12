#!/usr/bin/env python3
"""The DS game's joint animations (`.nsbca`, BCA0 files with a JNT0 block): the part of the format
that is established so far (docs/ULTIMATE-IMPORT.md, step У3.2, not closed).

Established on the ROM's 405 files (`python3 nsbca.py ROM.nds`):
  JNT0 -> dictionary of animations (named like the model) -> 'J\\0AC' record at `a`:
    +4 u16 frames, +6 u16 nodes, +8 u32 0, +0xC u32 and +0x10 u32 (offsets of the two rotation
    tables, from `a`), +0x14 u16 offset per node (from `a`).
  A node record: u32 tag (node index in the top byte, 24 flag bits below), then `words` u32 words
  up to the next node's record. The word count follows the flags exactly:
      words = 14 - 3*b1 - b3 - b4 - b5 - b6 - b8 - 6*b9     (bit n of the flags; bits 11..13 are
  always set together with 9 and add nothing)       [fitted on 12 960 records, no residual]
  A track that moves is a pair (info, offset): info's low half is the first frame, the high half
  holds the last frame in its low byte and mode bits on top (0x4000 / 0x6000 / 0x8000 / 0xA000);
  offset is into the animation's data pool. A track that does not move is one word.
NOT established: which flag bit names which of rotation / translation x,y,z / scale, the sample
format and the rotation-table encoding. `node_records` returns the raw words only.
"""
import os
import struct
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ndskit import lz, rom as ndsrom
import nsbmd
from nsbmd import u16, u32, NsbmdError


def parse(b):
    """[{name, frames, nodes: [{index, flags, words}], tables: (rot, rot2)}] for a BCA0 file."""
    if b[:4] != b'BCA0':
        raise NsbmdError('not a BCA0 file')
    out = []
    for k in range(u16(b, 14)):
        blk = u32(b, 16 + 4 * k)
        if b[blk:blk + 4] != b'JNT0':
            continue
        for name, e in nsbmd.dictionary(b, blk + 8):
            a = blk + u32(b, e)
            nsbmd.chk(b, a, 0x14)
            if b[a:a + 4] != b'J\0AC':
                raise NsbmdError(f'animation tag {b[a:a + 4]!r}')
            frames, n = u16(b, a + 4), u16(b, a + 6)
            limit = min(u32(b, a + 0xC), u32(b, a + 0x10))
            offs = [u16(b, a + 0x14 + 2 * i) for i in range(n)]
            ends = offs[1:] + [limit]
            nodes = []
            for i, (o, end) in enumerate(zip(offs, ends)):
                nsbmd.chk(b, a + o, 4)
                tag = u32(b, a + o)
                if tag >> 24 != i or end < o + 4 or (end - o) % 4:
                    raise NsbmdError(f'node record {i} of {name}')
                words = [u32(b, a + o + 4 + 4 * j) for j in range((end - o - 4) // 4)]
                nodes.append({'index': i, 'flags': tag & 0xFFFFFF, 'words': words})
            out.append({'name': name, 'frames': frames, 'nodes': nodes,
                        'tables': (u32(b, a + 0xC), u32(b, a + 0x10))})
    return out


def expected_words(flags):
    bit = lambda n: flags >> n & 1
    return 14 - 3 * bit(1) - bit(3) - bit(4) - bit(5) - bit(6) - bit(8) - 6 * bit(9) if not flags & 1 else 0


def rom_files(rom):
    for fid, name in rom.names.items():
        low = name.lower()
        if not (low.endswith('.nsbca') or low.endswith('.nsbca.z')):
            continue
        b = rom.file(fid)
        yield name, bytes(lz.decompress(b)) if low.endswith('.z') else bytes(b)


def main(argv):
    rom = ndsrom.Rom(open(argv[0], 'rb').read())
    files = anims = nodes = bad = 0
    for path, b in rom_files(rom):
        files += 1
        for a in parse(b):
            anims += 1
            for nd in a['nodes']:
                nodes += 1
                if len(nd['words']) != expected_words(nd['flags']):
                    bad += 1
    print(f'{files} files, {anims} animations, {nodes} node records, {bad} whose word count the flag formula misses')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
