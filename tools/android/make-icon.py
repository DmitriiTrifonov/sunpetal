#!/usr/bin/env python3
"""The app's icon, drawn in code (no picture to ship): a sun with eight petals.

    make-icon.py OUT_RES_DIR      writes mipmap-{mdpi,hdpi,xhdpi,xxhdpi,xxxhdpi}/ic_launcher.png
"""
import math
import os
import struct
import sys
import zlib


def png(path, w, h, rgba):
    def chunk(t, b):
        c = struct.pack('>I', len(b)) + t + b
        return c + struct.pack('>I', zlib.crc32(t + b))
    raw = b''.join(b'\0' + rgba[y * w * 4:(y + 1) * w * 4] for y in range(h))
    with open(path, 'wb') as f:
        f.write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0))
                + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b''))


def pixel(x, y):
    """Colour at (x, y) in 0..1 square: background, petals, the sun's disc."""
    # background: dusk, from deep violet at the top to warm orange at the bottom
    t = y
    bg = (0.20 + 0.75 * t, 0.10 + 0.35 * t, 0.38 - 0.10 * t)
    dx, dy = x - 0.5, y - 0.5
    r = math.hypot(dx, dy)
    a = math.atan2(dy, dx)
    # petals: eight, the radius of the outline a bump on each
    k = 8
    petal_r = 0.17 + 0.20 * (0.5 + 0.5 * math.cos(k * a)) ** 2
    if r < petal_r and r > 0.12:
        shade = 0.85 + 0.15 * math.cos(k * a)
        return (1.0 * shade, 0.80 * shade, 0.45 * shade)
    if r <= 0.12:
        return (1.0, 0.55, 0.12)
    return bg


def draw(size):
    ss = 3
    out = bytearray(size * size * 4)
    for py in range(size):
        for px in range(size):
            acc = [0.0, 0.0, 0.0]
            for j in range(ss):
                for i in range(ss):
                    c = pixel((px + (i + 0.5) / ss) / size, (py + (j + 0.5) / ss) / size)
                    for n in range(3):
                        acc[n] += c[n]
            o = (py * size + px) * 4
            out[o:o + 3] = bytes(max(0, min(255, int(v / (ss * ss) * 255))) for v in acc)
            out[o + 3] = 255
    return bytes(out)


def main(out):
    for name, size in (('mdpi', 48), ('hdpi', 72), ('xhdpi', 96), ('xxhdpi', 144), ('xxxhdpi', 192)):
        d = os.path.join(out, 'mipmap-' + name)
        os.makedirs(d, exist_ok=True)
        png(os.path.join(d, 'ic_launcher.png'), size, size, draw(size))


if __name__ == '__main__':
    main(sys.argv[1])
