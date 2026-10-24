#!/usr/bin/env python3
"""The Sunpetal data file (`.spdata`): the game's better-looking resources in one file, built on the
player's own machine from their own copies of the game and never shared.

Layout (little-endian):

    0   8 bytes   "SPDATA\\0" + version byte (1)
    8   u32       number of sections
    12  per section: 4-byte tag, u64 offset, u64 length          (20 bytes each)

Section `TEXS`, texture replacements (the keys are those of `texpack.rs`, see docs/BUILD.md):

    u32 count, then per entry: u64 data hash, u64 palette hash, u8 format, u16 width, u16 height
    (the DS texture's own), u64 absolute offset of its PNG, u32 its length (33 bytes each)

The PNGs follow. A reader that does not know a section's tag skips it (models will come as `MODL`).

    spdata.py info FILE            list the sections and count the textures
    spdata.py unpack FILE DIR      write the textures as PNGs named by key (a folder pack)
    spdata.py pack DIR FILE        make a file from a folder of PNGs named by key
"""
import os
import re
import struct
import sys

MAGIC = b'SPDATA\0\1'
NAME = re.compile(r'^(\d+)x(\d+)_(\d)_([0-9a-f]{16})_([0-9a-f]{16})\.png$')


def file_name(key):
    data, pal, fmt, w, h = key
    return f'{w}x{h}_{fmt}_{data:016x}_{pal:016x}.png'


def parse_name(name):
    m = NAME.match(name)
    if not m:
        return None
    w, h, fmt, data, pal = m.groups()
    return (int(data, 16), int(pal, 16), int(fmt), int(w), int(h))


def write(path, textures):
    """textures: iterable of (key, png_bytes); key = (data, pal, fmt, w, h). A key given twice keeps
    the last picture."""
    by_key = {}
    for key, png in textures:
        by_key[key] = png
    keys = sorted(by_key)
    head = len(MAGIC) + 4 + 20
    texs_index = 4 + 33 * len(keys)
    blob_at = head + texs_index
    index = bytearray(struct.pack('<I', len(keys)))
    blobs = bytearray()
    for key in keys:
        data, pal, fmt, w, h = key
        index += struct.pack('<QQBHHQI', data, pal, fmt, w, h, blob_at + len(blobs), len(by_key[key]))
        blobs += by_key[key]
    with open(path + '.part', 'wb') as f:
        f.write(MAGIC + struct.pack('<I', 1) + b'TEXS' + struct.pack('<QQ', head, texs_index + len(blobs)))
        f.write(index)
        f.write(blobs)
    os.replace(path + '.part', path)


def read(path):
    """-> (sections {tag: (offset, length)}, textures {key: (offset, length)})"""
    with open(path, 'rb') as f:
        if f.read(8) != MAGIC:
            raise ValueError('not a Sunpetal data file (or a newer version)')
        n, = struct.unpack('<I', f.read(4))
        sections = {}
        for _ in range(n):
            tag, off, ln = struct.unpack('<4sQQ', f.read(20))
            sections[tag.decode('latin-1')] = (off, ln)
        textures = {}
        if 'TEXS' in sections:
            f.seek(sections['TEXS'][0])
            count, = struct.unpack('<I', f.read(4))
            for _ in range(count):
                data, pal, fmt, w, h, off, ln = struct.unpack('<QQBHHQI', f.read(33))
                textures[(data, pal, fmt, w, h)] = (off, ln)
    return sections, textures


def png_of(path, entry):
    with open(path, 'rb') as f:
        f.seek(entry[0])
        return f.read(entry[1])


def main(argv):
    if len(argv) >= 2 and argv[0] == 'info':
        sections, textures = read(argv[1])
        for tag, (off, ln) in sections.items():
            print(f'{tag}: {ln} bytes at {off}')
        print(f'{len(textures)} textures')
    elif len(argv) == 3 and argv[0] == 'unpack':
        _, textures = read(argv[1])
        os.makedirs(argv[2], exist_ok=True)
        for key, entry in textures.items():
            with open(os.path.join(argv[2], file_name(key)), 'wb') as f:
                f.write(png_of(argv[1], entry))
        print(len(textures), 'pictures')
    elif len(argv) == 3 and argv[0] == 'pack':
        items = []
        for name in sorted(os.listdir(argv[1])):
            key = parse_name(name)
            if key:
                items.append((key, open(os.path.join(argv[1], name), 'rb').read()))
        write(argv[2], items)
        print(len(items), 'pictures')
    else:
        print(__doc__)
        return 2


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]) or 0)
