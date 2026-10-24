#!/usr/bin/env python3
"""Читатель архивов `LF` с PS2-диска (только чтение).

Формат (по разведке, см. docs/PS2-RECON.md):

    u16  'LF'
    u16  count
    u32  version (1)
    count x { u32 size, u32 id }     размер записи без выравнивания
    данные с 0x800, каждая запись выровнена по 0x800

Запись: DMA-тег (16 байт, QWC = (size-16)/16), дальше цепочка GIF-пакетов:
    GIF-тег A+D (4 регистра: BITBLTBUF, TRXPOS, TRXREG, TRXDIR) + GIF-тег IMAGE + данные.
То есть запись — это готовая загрузка картинки в видеопамять GS.

    lf.py ARCHIVE.BIN            список записей и картинок
    lf.py ARCHIVE.BIN OUTDIR     PNG для каждой картинки
"""
import os
import struct
import sys
import zlib

ALIGN = 0x800
SIG = bytes.fromhex('0400000000000010feffffffffffffff')   # GIF-тег: 4 регистра A+D

PSM_CT32, PSM_CT24, PSM_CT16, PSM_CT16S = 0x00, 0x01, 0x02, 0x0A
PSM_T8, PSM_T4 = 0x13, 0x14
PSM_NAMES = {0x00: 'CT32', 0x01: 'CT24', 0x02: 'CT16', 0x0A: 'CT16S',
             0x13: 'T8', 0x14: 'T4', 0x1B: 'T8H', 0x24: 'T4HL', 0x2C: 'T4HH'}


class Image:
    def __init__(self, entry, index, dbp, dbw, psm, w, h, data):
        self.entry, self.index = entry, index
        self.dbp, self.dbw, self.psm, self.w, self.h = dbp, dbw, psm, w, h
        self.data = data

    @property
    def name(self):
        return PSM_NAMES.get(self.psm, hex(self.psm))

    @property
    def is_clut(self):
        return self.psm in (PSM_CT32, PSM_CT16, PSM_CT16S) and self.w * self.h in (16, 256)


class Entry:
    def __init__(self, index, ident, data):
        self.index, self.id, self.data = index, ident, data
        self.images = []
        self.rest = 0          # байт, не разобранных как картинки
        self._parse()

    def _parse(self):
        d = self.data
        p = 16
        while p + 16 <= len(d):
            nxt = self._packet(d, p)
            if nxt is None:
                break
            p = nxt
        self.rest = len(d) - p
        if self.rest:
            self._scan(d, p)

    def _scan(self, d, p):
        """Записи-модели несут загрузки картинок внутри: ищем GIF-тег A+D по сигнатуре."""
        while True:
            p = d.find(SIG, p)
            if p < 0:
                return
            nxt = self._packet(d, p)
            p = nxt if nxt else p + 16

    def _packet(self, d, p):
        """Один GIF-пакет A+D + IMAGE по смещению p; возвращает конец или None."""
        if p + 16 > len(d):
            return None
        lo, hi = struct.unpack_from('<QQ', d, p)
        if lo == 0 and hi == 0:
            return None
        nloop, flg, nreg = lo & 0x7FFF, (lo >> 58) & 3, (lo >> 60) & 15
        p += 16
        if not (flg == 0 and (hi & 15) == 0xE and nreg == 1) or p + nloop * 16 + 16 > len(d):
            return None
        regs = {}
        for _ in range(nloop):
            val, addr = struct.unpack_from('<QQ', d, p)
            regs[addr & 0xFF] = val
            p += 16
        bb, trx = regs.get(0x50), regs.get(0x52)
        if bb is None or trx is None:
            return None
        lo2, _ = struct.unpack_from('<QQ', d, p)
        if (lo2 >> 58) & 3 != 2:
            return None
        n = (lo2 & 0x7FFF) * 16
        p += 16
        w, h = trx & 0xFFFFFFFF, trx >> 32
        if p + n > len(d):
            return None
        self.images.append(Image(self.index, len(self.images), bb >> 32 & 0x3FFF,
                                 bb >> 48 & 0x3F, bb >> 56 & 0x3F, w, h, d[p:p + n]))
        return p + n


class Archive:
    def __init__(self, raw):
        if raw[:2] != b'LF':
            raise ValueError('не LF')
        self.count, self.version = struct.unpack_from('<HI', raw, 2)
        self.entries = []
        o = ALIGN
        for i in range(self.count):
            size, ident = struct.unpack_from('<II', raw, 8 + 8 * i)
            self.entries.append(Entry(i, ident, raw[o:o + size]))
            o += (size + ALIGN - 1) & ~(ALIGN - 1)
        self.tail = len(raw) - o


def clut_colors(img):
    """Палитра как список (r,g,b,a); для 256 цветов снимается перестановка CSM1."""
    d = img.data
    n = img.w * img.h
    if img.psm == PSM_CT32:
        cols = [(d[4 * i], d[4 * i + 1], d[4 * i + 2], min(255, d[4 * i + 3] * 2)) for i in range(n)]
    else:
        cols = [px16(struct.unpack_from('<H', d, 2 * i)[0]) for i in range(n)]
    if n == 256:
        cols = [cols[(i & ~0x18) | ((i & 8) << 1) | ((i & 16) >> 1)] for i in range(256)]
    return cols


def px16(v):
    r, g, b = v & 31, v >> 5 & 31, v >> 10 & 31
    return (r << 3 | r >> 2, g << 3 | g >> 2, b << 3 | b >> 2, 255 if v & 0x8000 else 0)


def to_rgba(img, clut=None):
    d, w, h = img.data, img.w, img.h
    out = bytearray(w * h * 4)
    if img.psm == PSM_CT32:
        for i in range(w * h):
            out[4 * i:4 * i + 3] = d[4 * i:4 * i + 3]
            out[4 * i + 3] = min(255, d[4 * i + 3] * 2)
    elif img.psm == PSM_CT24:
        for i in range(w * h):
            out[4 * i:4 * i + 3] = d[3 * i:3 * i + 3]
            out[4 * i + 3] = 255
    elif img.psm in (PSM_CT16, PSM_CT16S):
        for i in range(w * h):
            out[4 * i:4 * i + 4] = bytes(px16(d[2 * i] | d[2 * i + 1] << 8))
    elif img.psm in (PSM_T8, PSM_T4):
        if clut is None:
            clut = [(i * 17, i * 17, i * 17, 255) for i in range(16)] if img.psm == PSM_T4 \
                else [(i, i, i, 255) for i in range(256)]
        for i in range(w * h):
            if img.psm == PSM_T8:
                v = d[i]
            else:
                v = d[i >> 1] >> (4 * (i & 1)) & 15
            out[4 * i:4 * i + 4] = bytes(clut[v])
    else:
        return None
    return bytes(out)


def png_bytes(w, h, rgba):
    def chunk(t, b):
        c = struct.pack('>I', len(b)) + t + b
        return c + struct.pack('>I', zlib.crc32(t + b))
    raw = b''.join(b'\0' + rgba[y * w * 4:(y + 1) * w * 4] for y in range(h))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(raw, 6)) + chunk(b'IEND', b''))


def write_png(path, w, h, rgba):
    with open(path, 'wb') as f:
        f.write(png_bytes(w, h, rgba))


def read_png(path):
    """(w, h, rgba) для PNG 8 бит (серый, серый+альфа, RGB, RGBA), без палитры и чересстрочности."""
    b = open(path, 'rb').read()
    i, idat = 8, b''
    while i < len(b):
        n = struct.unpack('>I', b[i:i + 4])[0]
        t = b[i + 4:i + 8]
        if t == b'IHDR':
            w, h = struct.unpack('>II', b[i + 8:i + 16])
            ct = b[i + 17]
        elif t == b'IDAT':
            idat += b[i + 8:i + 8 + n]
        i += 12 + n
    bpp = {0: 1, 2: 3, 4: 2, 6: 4}[ct]
    raw = zlib.decompress(idat)
    st = w * bpp
    prev = bytearray(st)
    out = bytearray()
    pos = 0
    for _ in range(h):
        f = raw[pos]
        row = bytearray(raw[pos + 1:pos + 1 + st])
        pos += 1 + st
        for k in range(st):
            a = row[k - bpp] if k >= bpp else 0
            up = prev[k]
            c = prev[k - bpp] if k >= bpp else 0
            if f == 1:
                row[k] = (row[k] + a) & 255
            elif f == 2:
                row[k] = (row[k] + up) & 255
            elif f == 3:
                row[k] = (row[k] + ((a + up) >> 1)) & 255
            elif f == 4:
                pa, pb, pc = abs(up - c), abs(a - c), abs(a + up - 2 * c)
                row[k] = (row[k] + (a if pa <= pb and pa <= pc else up if pb <= pc else c)) & 255
        prev = row
        for x in range(w):
            q = row[x * bpp:(x + 1) * bpp]
            out += bytes((q[0], q[0], q[0], 255)) if bpp == 1 else bytes((q[0], q[0], q[0], q[1])) if bpp == 2 \
                else bytes((q[0], q[1], q[2], 255)) if bpp == 3 else bytes(q)
    return w, h, bytes(out)


def pair_clut(entry, img):
    """Палитра, загруженная в той же записи после картинки."""
    if img.psm not in (PSM_T8, PSM_T4):
        return None
    want = 256 if img.psm == PSM_T8 else 16
    for o in entry.images[img.index + 1:]:
        if o.is_clut and o.w * o.h == want:
            return clut_colors(o)
    return None


def main(argv):
    raw = open(argv[0], 'rb').read()
    out = argv[1] if len(argv) > 1 else None
    a = Archive(raw)
    print(f'LF: записей {a.count}, версия {a.version}, хвост {a.tail}')
    for e in a.entries:
        print(f'  [{e.index:3}] id={e.id:#x} {len(e.data):#x} байт, картинок {len(e.images)}, не разобрано {e.rest:#x}')
        for im in e.images:
            print(f'        #{im.index} {im.w}x{im.h} {im.name} dbp={im.dbp:#x} dbw={im.dbw}'
                  f'{" (палитра)" if im.is_clut else ""}')
            if out and not im.is_clut:
                rgba = to_rgba(im, pair_clut(e, im))
                if rgba:
                    os.makedirs(out, exist_ok=True)
                    write_png(os.path.join(out, f'{e.index:03}_{im.index}_{im.w}x{im.h}_{im.name}.png'),
                              im.w, im.h, rgba)


if __name__ == '__main__':
    main(sys.argv[1:])
