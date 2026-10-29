#!/usr/bin/env python3
"""A tiny PNG reader for 8-bit RGB/RGBA non-interlaced PNGs (the screenshots this project
writes): enough to look at pixels without PIL. Also a writer for the parity diff maps
(docs/GPU-BACKEND.md, этап 0's harness needs one later)."""
import struct, zlib

def read(path):
    d = open(path, 'rb').read()
    assert d[:8] == b'\x89PNG\r\n\x1a\n', 'not a PNG'
    o, idat, hdr = 8, [], None
    while o < len(d):
        (n,) = struct.unpack_from('>I', d, o)
        tag = d[o+4:o+8]
        body = d[o+8:o+8+n]
        if tag == b'IHDR':
            hdr = struct.unpack_from('>IIBB', body)
        elif tag == b'IDAT':
            idat.append(body)
        elif tag == b'IEND':
            break
        o += 12 + n
    w, h, bit, color = hdr
    assert bit == 8 and color in (2, 6), f'bit {bit} color {color}: need 8-bit RGB(A)'
    raw = zlib.decompress(b''.join(idat))
    stride = w * (4 if color == 6 else 3)
    px = bytearray()
    prev = bytearray(stride)
    for y in range(h):
        row = bytearray(raw[y*(stride+1)+1:(y+1)*(stride+1)])
        f = raw[y*(stride+1)]
        if f == 1:
            for i in range(3, stride):
                row[i] = (row[i] + row[i-3]) & 255
        elif f == 2:
            for i in range(stride):
                row[i] = (row[i] + prev[i]) & 255
        elif f == 3:
            for i in range(stride):
                row[i] = (row[i] + ((row[i-3] if i >= 3 else 0) + prev[i]) // 2) & 255
        elif f == 4:
            for i in range(stride):
                a = row[i-3] if i >= 3 else 0
                b = prev[i]
                c = prev[i-3] if i >= 3 else 0
                p = a + b - c
                pa, pb, pc = abs(p-a), abs(p-b), abs(p-c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                row[i] = (row[i] + pr) & 255
        px += row
        prev = row
    return w, h, color == 6, bytes(px)

def write(path, w, h, px, rgba=False):
    """px: w*h RGB or RGBA bytes, one row after the other."""
    raw = bytearray()
    stride = w * (4 if rgba else 3)
    for y in range(h):
        raw.append(0)  # filter: none
        raw += px[y*stride:(y+1)*stride]
    def chunk(tag, body):
        c = struct.pack('>I', len(body)) + tag + body
        return c + struct.pack('>I', zlib.crc32(tag + body) & 0xffffffff)
    out = b'\x89PNG\r\n\x1a\n'
    out += chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6 if rgba else 2, 0, 0, 0))
    out += chunk(b'IDAT', zlib.compress(bytes(raw), 6))
    out += chunk(b'IEND', b'')
    open(path, 'wb').write(out)

if __name__ == '__main__':
    import sys
    if sys.argv[1] == 'read':
        w, h, rgba, px = read(sys.argv[2])
        print(f'{sys.argv[2]}: {w}x{h} {"RGBA" if rgba else "RGB"}')
        x0, y0, x1, y1 = (int(v) for v in sys.argv[3:7])
        row = px[y0*w*(4 if rgba else 3):(y1)*w*(4 if rgba else 3)]
        for y in range(y0, y1):
            print(' '.join(f'{row[y*w*(4 if rgba else 3)+x*(4 if rgba else 3)]:3d}' for x in range(x0, x1)))
