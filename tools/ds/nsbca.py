#!/usr/bin/env python3
"""The DS game's joint animations (`.nsbca`, BCA0 files with a JNT0 block), read completely
(docs/ULTIMATE-IMPORT.md, step У3.2).

On the ROM's 405 files (`python3 nsbca.py ROM.nds`):
  JNT0 -> dictionary of animations (named like the model) -> 'J\\0AC' record at `a`:
    +4 u16 frames, +6 u16 nodes, +8 u32 0, +0xC u32 and +0x10 u32 (offsets of the two rotation
    tables, from `a`), +0x14 u16 offset per node (from `a`).
  A node record: u32 tag (node index in the top byte, 24 flag bits below), then `words` u32 words
  up to the next node's record. The word count follows the flags exactly:
      words = 14 - 3*b1 - b3 - b4 - b5 - b6 - b8 - 6*b9     (bit n of the flags; bits 11..13 are
  always set together with 9 and add nothing)       [fitted on 12 960 records, no residual]
  What the words mean (translation, rotation, samples, the two rotation tables): see `tracks` and the
  comment above it. Checked against the model's rest pose (M0101, a two-frame pose, on `C001`: all
  translations equal to the model's, rotations a few degrees off) and by the rotations being unit
  matrices (392 564 node-frames of the ROM, worst deviation 0.022 = the table's quantisation).
Not decoded: scale (never present in the ROM: bit 9 is set on every record) and the interpolation the
game applies between samples (`pose` gives the sample at or before the frame).
"""
import math
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
            out.append({'name': name, 'at': a, 'frames': frames, 'nodes': nodes,
                        'tables': (u32(b, a + 0xC), u32(b, a + 0x10))})
    return out


# ---- decoding (У3.2, established 2026-11-13 against the model's rest pose, see the journal) ----------
#
# Flags of a node record (bit n): 0 whole identity; 1 translation identity (x, y, z const 0);
# 3, 4, 5 translation x, y, z is one const word (fx32 / 4096) instead of an (info, offset) pair;
# 6 and 8 rotation identity / const (one word, an index; identity: no word); 9 scale identity (6 words
# of scale that are absent). Words, in order: tx ty tz (1 or 2 words each), rotation (1 or 2), scale.
# A pair's info: low half first frame; the high half's low 13 bits are the last frame; mode bits on top of
# the high half: 0x4000 samples every 2 frames, 0x8000 every 4 frames, 0x2000 s16 samples instead of s32
# (translation; fx / 4096). The offset (from the animation record) leads to the samples of the track.
# A rotation sample / const word is an index: bit 15 set -> table 0 (`J\0AC+0xC`, 3 x s16 = (info, a, b), a
# pivot rotation, same cells as the model's pivot nodes: cell = info & 15, 0x10 / 0x20 / 0x40 negate
# as the model's 0x100 / 0x200 / 0x400, a and b over 4096); else table 1 (`+0x10`, 5 x s16 = A B C D E over
# 32768: the first row is (A, B, C), the second (D, E, F) with F from the unit length of the row, its sign the lowest bit of E).


def tracks(flags, words):
    """Splits a node record's words into ('t', [3 tracks]), ('r', track | None), identity flags:
    each track is ('const', value_word) or ('anim', info, offset)."""
    if flags & 1:
        return [None, None, None], None
    pos = 0
    t = []
    for k in (3, 4, 5):
        if flags >> 1 & 1:
            t.append(('const', 0))
        elif flags >> k & 1:
            t.append(('const', words[pos]))
            pos += 1
        else:
            t.append(('anim', words[pos], words[pos + 1]))
            pos += 2
    if flags >> 6 & 1:
        r = None
    elif flags >> 8 & 1:
        r = ('const', words[pos])
    else:
        r = ('anim', words[pos], words[pos + 1])
    return t, r


def sample_shape(info):
    """(shift, s16, last frame): the step shift (0, 1, 2), whether the samples are s16, the last frame."""
    hi = info >> 16
    return (2 if hi & 0x8000 else 1 if hi & 0x4000 else 0), bool(hi & 0x2000), hi & 0x1FFF


def sample_count(frames, info):
    """Samples a track stores: up to the last frame, one more when it is not on a sample (the last step)."""
    shift = sample_shape(info)[0]
    last = frames - 1
    return (last >> shift) + (1 if last & ((1 << shift) - 1) else 0) + 1


def pivot_entry(info, a, b):
    return nsbmd.pivot_rotation((info & 15) << 4 | (info & 0x10) << 4 | (info & 0x20) << 4 | (info & 0x40) << 4,
                                a / 4096, b / 4096)


def rot5_entry(a, b, c, d, e):
    """Rows (A, B, C), (D, E, F), their cross product; the values over 32768. The lowest bit of E is the sign
    of F (checked on 38 000 table entries); F's size follows from the second row being a unit vector."""
    r0 = [a / 32768, b / 32768, c / 32768]
    neg = e & 1
    d, e = d / 32768, e / 32768
    f = math.sqrt(max(0.0, 1.0 - d * d - e * e))
    r1 = [d, e, -f if neg else f]
    r2 = [r0[1] * r1[2] - r0[2] * r1[1], r0[2] * r1[0] - r0[0] * r1[2], r0[0] * r1[1] - r0[1] * r1[0]]
    return [r0, r1, r2]


def rotation(b, anim_at, tables, idx):
    """The rotation matrix an index names: bit 15 -> the pivot table (+0xC), else the five-value table (+0x10)."""
    t0, t1 = tables
    if idx & 0x8000:
        return pivot_entry(*struct.unpack_from('<3h', b, anim_at + t0 + 6 * (idx & 0x7FFF)))
    return rot5_entry(*struct.unpack_from('<5h', b, anim_at + t1 + 10 * idx))


def pose(b, anim, node, frame, anim_at, lerp=False):
    """(T, R) of one node at a frame. Default: the sample at or before an integer frame. `lerp`: a fractional
    frame, translation and rotation matrix element-wise interpolated between neighbouring samples (what the
    game's NNS does for tracks stepping 2 or 4 frames; checked against the game in `check_pose.py`)."""
    nd = anim['nodes'][node]
    t, r = tracks(nd['flags'], nd['words'])
    ident = [[1.0, 0, 0], [0, 1.0, 0], [0, 0, 1.0]]
    if t[0] is None:
        return (0.0, 0.0, 0.0), ident

    def at(trk, frame_i, kind):
        """sample index and weight of the next one for a track"""
        shift = sample_shape(trk[1])[0]
        n = sample_count(anim['frames'], trk[1])
        pos = frame / (1 << shift) if lerp else frame_i >> shift
        i = min(int(pos), n - 1)
        return i, (pos - i if lerp and i + 1 < n else 0.0), n

    def val(trk):
        if trk[0] == 'const':
            w = trk[1]
            return (w - (1 << 32) if w >> 31 else w) / 4096
        s16 = sample_shape(trk[1])[1]
        i, f, _ = at(trk, int(frame), 't')
        rd = (lambda k: struct.unpack_from('<h', b, anim_at + trk[2] + 2 * k)[0] / 4096) if s16 else (lambda k: struct.unpack_from('<i', b, anim_at + trk[2] + 4 * k)[0] / 4096)
        return rd(i) * (1 - f) + rd(i + 1) * f if f else rd(i)
    tv = tuple(val(x) for x in t)
    if r is None:
        return tv, ident
    if r[0] == 'const':
        return tv, rotation(b, anim_at, anim['tables'], r[1])
    i, f, _ = at(r, int(frame), 'r')
    idx = lambda k: struct.unpack_from('<H', b, anim_at + r[2] + 2 * k)[0]
    r0 = rotation(b, anim_at, anim['tables'], idx(i))
    if not f:
        return tv, r0
    r1 = rotation(b, anim_at, anim['tables'], idx(i + 1))
    m = [[r0[a][c] * (1 - f) + r1[a][c] * f for c in range(3)] for a in range(3)]
    m = [[v / (sum(x * x for x in row) ** 0.5) for v in row] for row in m]  # the game's rows stay unit (checked, check_pose.py)
    return tv, m


def world_matrices(model, b, anim, frame, lerp=False):
    """The model's node world matrices (row-vector 4 x 3, as `nsbmd.Model.nodes[i]['world']`) with the
    animation's translation and rotation at `frame`; the scale stays the model's. Nodes the animation has
    no record for (more nodes than records) keep the model's own local matrix."""
    done = {}

    def world(i, depth=0):
        if i in done:
            return done[i]
        nd = model.nodes[i]
        if i < len(anim['nodes']):
            t, r = pose(b, anim, i, frame, anim['at'], lerp)
            local = nsbmd.local_matrix(t, r, nd['s'])
        else:
            local = nd['local']
        p = nd['parent']
        ok = p is not None and p != i and p != 0xFF and p < len(model.nodes) and depth < len(model.nodes)
        done[i] = nsbmd.mul(local, world(p, depth + 1)) if ok else local
        return done[i]
    return [world(i) for i in range(len(model.nodes))]


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
    files = anims = nodes = bad = frames = skew = 0
    worst = 0.0
    for path, b in rom_files(rom):
        files += 1
        for a in parse(b):
            anims += 1
            for nd in a['nodes']:
                nodes += 1
                if len(nd['words']) != expected_words(nd['flags']):
                    bad += 1
            for i in range(len(a['nodes'])):
                for f in range(a['frames']):
                    _, r = pose(b, a, i, f, a['at'])
                    e = max(abs(sum(r[p][k] * r[q][k] for k in range(3)) - (p == q)) for p in range(3) for q in range(3))
                    worst = max(worst, e)
                    skew += e > 0.03
                    frames += 1
    print(f'{files} files, {anims} animations, {nodes} node records, {bad} whose word count the flag formula misses; '
          f'{frames} node-frames decoded, {skew} rotations off unit by more than 0.03 (worst {worst:.3f})')
    return 1 if bad or skew else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
