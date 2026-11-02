#!/usr/bin/env python3
"""VRAM GS из сохранения PCSX2 в PNG (только чтение, без PIL и numpy).

    vram.py STATE OUT.png [--pages N] [--off N]     вся VRAM как плоскость 1024 px, с разворотом свизла
    vram.py STATE OUT.png --raw                           слова подряд, без разворота (для сверки)

PSMCT32: страница 8 КБ = 64×32 px, блок 8×8 px (32 блока в странице), столбцы по 2 px.
Базовая страница: 1024 px шириной = 16 страниц в строке.
"""
import struct
import sys
import zlib
import os

sys.path.insert(0, os.path.dirname(__file__))
import p2s  # noqa: E402

BLOCK32 = [
    [0, 1, 4, 5, 16, 17, 20, 21], [2, 3, 6, 7, 18, 19, 22, 23],
    [8, 9, 12, 13, 24, 25, 28, 29], [10, 11, 14, 15, 26, 27, 30, 31],
]
# слово внутри блока PSMCT32 по (y, x) в блоке 8×8
COL32 = [
    [0, 1, 4, 5, 8, 9, 12, 13], [2, 3, 6, 7, 10, 11, 14, 15],
    [16, 17, 20, 21, 24, 25, 28, 29], [18, 19, 22, 23, 26, 27, 30, 31],
    [32, 33, 36, 37, 40, 41, 44, 45], [34, 35, 38, 39, 42, 43, 46, 47],
    [48, 49, 52, 53, 56, 57, 60, 61], [50, 51, 54, 55, 58, 59, 62, 63],
]


def png(path, w, h, rgb):
    raw = b"".join(b"\0" + rgb[y * w * 3:(y + 1) * w * 3] for y in range(h))

    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d))
    open(path, "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
                           + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))


def load_vram(state, off=None):
    gs = p2s.read(p2s.entries(state), "GS.bin")
    n = 4 * 1024 * 1024
    if off is None:
        off = 509  # гипотеза: 509 байт заголовка, затем VRAM (разворот свизла проверен не до конца)
    return gs[off:off + n]


def ct32_pixel_addr(x, y, base_pages=16):
    """Адрес слова PSMCT32 по (x, y), ширина буфера base_pages*64 px."""
    page = (y // 32) * base_pages + (x // 64)
    block = BLOCK32[(y % 32) // 8][(x % 64) // 8]
    return page * 2048 + block * 64 + COL32[y % 8][x % 8]


def main():
    a = sys.argv[1:]
    if len(a) < 2:
        raise SystemExit(__doc__)
    off = int(a[a.index("--off") + 1]) if "--off" in a else None
    v = load_vram(a[0], off)
    words = struct.unpack("<1048576I", v)
    out = bytearray(1024 * 1024 * 3)
    raw = "--raw" in a
    pages = int(a[a.index("--pages") + 1]) if "--pages" in a else 16  # ширина буфера в страницах по 64 px
    for y in range(1024):
        for x in range(1024):
            w = words[y * 1024 + x] if raw else words[ct32_pixel_addr(x, y, pages)]
            i = (y * 1024 + x) * 3
            out[i], out[i + 1], out[i + 2] = w & 255, (w >> 8) & 255, (w >> 16) & 255
    png(a[1], 1024, 1024, bytes(out))


if __name__ == "__main__":
    main()
