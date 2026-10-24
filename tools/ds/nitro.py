#!/usr/bin/env python3
"""The DS game's 3D textures, read from the ROM's files alone (no need to play the game).

`.nsbmd` models and `.nsbtx` texture archives (also LZ-compressed, `.z`) hold a TEX0 block: the
textures (format, size, texels) and the palettes. This module reads them, works out the key the
runtime gives each texture (`texpack.rs`: hashes of the texels and of the palette, with the format
and size, so a picture made for a key replaces that texture) and decodes the texture to RGBA at its
own size, for matching against the PS2 version's pictures.

    nitro.py ROM.nds              how many textures and keys there are
    nitro.py ROM.nds DUMP_DIR     and whether every picture the game dumped (`--dump-textures`) is among them
"""
import os
import struct
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))
from ndskit import lz, rom as ndsrom

FNV_BASIS = 0xCBF29CE484222325
FNV_PRIME = 0x100000001B3
MASK = (1 << 64) - 1
BITS = {1: 8, 2: 2, 3: 4, 4: 8, 5: 2, 6: 8, 7: 16}


def fnv(data, h):
    for b in data:
        h = ((h ^ b) * FNV_PRIME) & MASK
    return h


class Texture:
    def __init__(self, name, fmt, w, h, c0, data, info):
        self.name, self.fmt, self.w, self.h, self.c0 = name, fmt, w, h, c0
        self.data, self.info = data, info      # info: the compressed format's per-tile words


class Tex0:
    def __init__(self, textures, palettes):
        self.textures = textures
        self.palettes = palettes               # [(name, bytes)]


def _dict(b, at):
    """A NITRO dictionary at `at`: [(name, entry bytes)]."""
    n = b[at + 1]
    entries_at = at + struct.unpack_from('<H', b, at + 6)[0]
    esize = struct.unpack_from('<H', b, entries_at)[0]
    first = entries_at + 4
    names_at = first + n * esize
    out = []
    for i in range(n):
        name = bytes(b[names_at + 16 * i:names_at + 16 * i + 16]).split(b'\0')[0].decode('latin1')
        out.append((name, bytes(b[first + i * esize:first + (i + 1) * esize])))
    return out


def parse_tex0(b, at):
    """The TEX0 block at `at` of the file `b`."""
    (size, _, tex_size, tex_info, _, tex_data, _, comp_size, comp_info, _, comp_data, comp_info_data, _,
     pal_size, pal_info, pal_data) = struct.unpack_from('<IIHHIIIHHIIIIIII', b, at + 4)
    pal_size &= 0xFFFF
    textures = []
    for name, e in _dict(b, at + tex_info):
        off, params = struct.unpack_from('<HH', e, 0)
        fmt = (params >> 10) & 7
        w, h = 8 << ((params >> 4) & 7), 8 << ((params >> 7) & 7)
        c0 = (params >> 13) & 1
        n = w * h * BITS.get(fmt, 8) // 8
        if fmt == 5:
            o = at + comp_data + off * 8
            info_at = at + comp_info_data + (off * 8) // 2
            t = Texture(name, fmt, w, h, c0, bytes(b[o:o + n]), bytes(b[info_at:info_at + (w // 4) * (h // 4) * 2]))
            if len(t.data) == n and len(t.info) == (w // 4) * (h // 4) * 2:
                textures.append(t)
        else:
            o = at + tex_data + off * 8
            t = Texture(name, fmt, w, h, c0, bytes(b[o:o + n]), None)
            if len(t.data) == n:      # a texture cut off by its file's end is not one the game draws
                textures.append(t)
    pal_entries = _dict(b, at + pal_info)
    end = pal_size * 8
    palettes = []
    for name, e in pal_entries:
        o = struct.unpack_from('<H', e, 0)[0] * 8
        # from the palette to the end of the block: in memory the palettes follow one another, and
        # a compressed texture's window can run on into the next
        palettes.append((name, bytes(b[at + pal_data + o:at + pal_data + end])))
    return Tex0(textures, palettes)


def tex0_of(b):
    """The TEX0 blocks of a BMD0 or BTX0 file."""
    if b[:4] not in (b'BMD0', b'BTX0'):
        return []
    nblocks = struct.unpack_from('<H', b, 14)[0]
    out = []
    for off in struct.unpack_from('<%dI' % nblocks, b, 16):
        if b[off:off + 4] == b'TEX0':
            out.append(parse_tex0(b, off))
    return out


def key(t, pal):
    """The runtime's key for texture `t` read through palette bytes `pal` (starting at the palette):
    (data hash, palette hash, format, width, height)."""
    data = fnv(t.data, fnv([t.fmt, t.c0 if 2 <= t.fmt <= 4 else 0, t.w >> 3, t.h >> 3], FNV_BASIS))
    def window(frm, n):
        return fnv((pal[frm + i] if frm + i < len(pal) else 0 for i in range(n)), FNV_BASIS)
    if t.fmt == 5:
        words = [struct.unpack_from('<H', t.info, 2 * i)[0] for i in range(len(t.info) // 2)]
        data = fnv(t.info, data)
        idx = [w & 0x3FFF for w in words]
        palette = window(min(idx) * 4, (max(idx) - min(idx)) * 4 + 8) if idx else 0
    elif t.fmt in (1, 2, 3, 4, 6):
        d = t.data
        if t.fmt == 1:
            top = max(b & 31 for b in d)
        elif t.fmt == 2:
            top = max(max(b & 3, b >> 2 & 3, b >> 4 & 3, b >> 6) for b in d)
        elif t.fmt == 3:
            top = max(max(b & 15, b >> 4) for b in d)
        elif t.fmt == 4:
            top = max(d)
        else:
            top = max(b & 7 for b in d)
        palette = window(0, (top + 1) * 2)
    else:
        palette = 0
    return (data, palette, t.fmt, t.w, t.h)


def _c(c):
    """A 5-bit colour as the runtime draws it (6 bits, then 8)."""
    v = c * 2 + 1 if c else 0
    return v << 2 | v >> 4


def _rgb(c555):
    return _c(c555 & 31), _c(c555 >> 5 & 31), _c(c555 >> 10 & 31)


def decode(t, pal):
    """RGBA bytes of the texture at its own size, as the runtime's `decode` makes them."""
    w, h, d = t.w, t.h, t.data
    out = bytearray(w * h * 4)
    P = lambda i: struct.unpack_from('<H', pal, 2 * i)[0] if 2 * i + 2 <= len(pal) else 0
    def put(x, y, r, g, b, a):
        o = (y * w + x) * 4
        out[o:o + 4] = bytes((r, g, b, a))
    for y in range(h):
        for x in range(w):
            if t.fmt == 1:
                v = d[y * w + x]
                r, g, b = _rgb(P(v & 31))
                a3 = v >> 5
                put(x, y, r, g, b, (a3 * 4 + a3 // 2) * 255 // 31)
            elif t.fmt in (2, 3, 4):
                if t.fmt == 2:
                    v = d[(y * w + x) // 4] >> (2 * (x & 3)) & 3
                elif t.fmt == 3:
                    v = d[(y * w + x) // 2]
                    v = v >> 4 if x & 1 else v & 15
                else:
                    v = d[y * w + x]
                if v == 0 and t.c0:
                    continue
                r, g, b = _rgb(P(v))
                put(x, y, r, g, b, 255)
            elif t.fmt == 6:
                v = d[y * w + x]
                r, g, b = _rgb(P(v & 7))
                put(x, y, r, g, b, (v >> 3) * 255 // 31)
            elif t.fmt == 7:
                c = struct.unpack_from('<H', d, (y * w + x) * 2)[0]
                r, g, b = _rgb(c)
                put(x, y, r, g, b, 255 if c & 0x8000 else 0)
            elif t.fmt == 5:
                tile = (y >> 2) * (w >> 2) + (x >> 2)
                idx = d[tile * 4 + (y & 3)] >> (2 * (x & 3)) & 3
                info = struct.unpack_from('<H', t.info, tile * 2)[0]
                base = (info & 0x3FFF) * 2          # in colours
                mode = info >> 14
                col = lambda i: _rgb(P(base + i))
                c0, c1 = col(0), col(1)
                avg = lambda a, b, wa, wb: tuple((a[k] * wa + b[k] * wb) // 8 for k in range(3))
                if idx == 0:
                    c = c0
                elif idx == 1:
                    c = c1
                elif idx == 2:
                    c = col(2) if mode in (0, 2) else avg(c0, c1, 4, 4) if mode == 1 else avg(c0, c1, 5, 3)
                else:
                    if mode in (0, 1):
                        continue
                    c = col(3) if mode == 2 else avg(c0, c1, 3, 5)
                put(x, y, c[0], c[1], c[2], 255)
    return bytes(out)


def rom_files(rom):
    """(path, bytes) of the ROM's 3D files, decompressed."""
    for fid, name in rom.names.items():
        low = name.lower()
        base = low[:-2] if low.endswith('.z') else low
        if not (base.endswith('.nsbmd') or base.endswith('.nsbtx')):
            continue
        b = rom.file(fid)
        if low.endswith('.z'):
            b = bytes(lz.decompress(b))
        yield name, bytes(b)


def catalog(rom):
    """Every (key, file, texture, palette bytes) the ROM's models and texture archives can give.
    A palette is taken from the same TEX0 block: the one named like the texture (`name` or
    `name_pl`) if there is one, else every palette of the block."""
    for path, b in rom_files(rom):
        for blk in tex0_of(b):
            for t in blk.textures:
                if t.fmt == 7:
                    yield key(t, b''), path, t, b''
                    continue
                named = [p for n, p in blk.palettes if n in (t.name, t.name + '_pl', t.name + '_PL')]
                # the game takes "colour 0 is transparent" from the material, not the texture: both
                for c0 in ((0, 1) if 2 <= t.fmt <= 4 else (t.c0,)):
                    v = t if c0 == t.c0 else Texture(t.name, t.fmt, t.w, t.h, c0, t.data, t.info)
                    for pal in (named or [p for _, p in blk.palettes]):
                        yield key(v, pal), path, v, pal


def main(argv):
    rom = ndsrom.Rom(open(argv[0], 'rb').read())
    textures = keys = 0
    seen = set()
    files = set()
    for k, path, t, pal in catalog(rom):
        keys += 1
        seen.add(k)
        files.add(path)
    for path, b in rom_files(rom):
        for blk in tex0_of(b):
            textures += len(blk.textures)
    print(f'{len(files)} files with textures, {textures} textures, {keys} (texture, palette) pairs, {len(seen)} distinct keys')
    if len(argv) > 1:
        import re
        names = [n for n in os.listdir(argv[1]) if n.endswith('.png')]
        missing = []
        for n in names:
            m = re.match(r'^(\d+)x(\d+)_(\d)_([0-9a-f]{16})_([0-9a-f]{16})\.png$', n)
            k = (int(m[4], 16), int(m[5], 16), int(m[3]), int(m[1]), int(m[2]))
            if k not in seen:
                missing.append(n)
        print(f'{len(names) - len(missing)} of {len(names)} dumped pictures are in the catalog')
        for n in missing[:10]:
            print('  not found:', n)


if __name__ == '__main__':
    main(sys.argv[1:])
