#!/usr/bin/env python3
"""Samples for 8е.4б: icons of the device made from the OAM pieces of a `dump2d` folder, enlarged x4 by
several methods (stdlib only; nothing from the game goes into the repository).

    samples-2d.py SRC_DIR OUT_DIR

SRC_DIR holds `index.csv` and the PNGs `sunpetal-debug dump2d` wrote.
"""
import csv, os, struct, sys, zlib

K = 4  # the enlargement

def read_png(path):
    d = open(path, 'rb').read(); i = 8; idat = b''
    while i < len(d):
        n, t = struct.unpack('>I4s', d[i:i + 8]); c = d[i + 8:i + 8 + n]
        if t == b'IHDR': w, h, depth, ct = struct.unpack('>IIBB', c[:10]); assert (depth, ct) == (8, 6), path
        if t == b'IDAT': idat += c
        i += 12 + n
    raw = zlib.decompress(idat); st = w * 4; prev = bytearray(st); o = 0; px = []
    for y in range(h):
        f = raw[o]; line = bytearray(raw[o + 1:o + 1 + st]); o += 1 + st
        for x in range(st):
            a = line[x - 4] if x >= 4 else 0; b = prev[x]; c = prev[x - 4] if x >= 4 else 0
            if f == 1: line[x] = (line[x] + a) & 255
            elif f == 2: line[x] = (line[x] + b) & 255
            elif f == 3: line[x] = (line[x] + (a + b) // 2) & 255
            elif f == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        px.append([tuple(line[4 * x:4 * x + 4]) for x in range(w)]); prev = line
    return px

def write_png(path, px):
    h, w = len(px), len(px[0])
    raw = b''.join(b'\0' + bytes(v for p in row for v in p) for row in px)
    def chunk(t, c): return struct.pack('>I', len(c)) + t + c + struct.pack('>I', zlib.crc32(t + c))
    open(path, 'wb').write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b''))

CLEAR = (0, 0, 0, 0)

def compose(src, pieces):
    """The pieces (x, y, png) drawn lowest OAM index on top, cropped to their box."""
    x0 = min(p[0] for p in pieces); y0 = min(p[1] for p in pieces)
    x1 = max(p[0] + len(p[2][0]) for p in pieces); y1 = max(p[1] + len(p[2]) for p in pieces)
    out = [[CLEAR] * (x1 - x0) for _ in range(y1 - y0)]
    for x, y, im in reversed(pieces):
        for j, row in enumerate(im):
            for i, p in enumerate(row):
                if p[3]: out[y - y0 + j][x - x0 + i] = p
    return out

def nearest(im, k=K):
    return [[p for p in row for _ in range(k)] for row in im for _ in range(k)]

def bilinear(im, k=K):
    h, w = len(im), len(im[0])
    def at(x, y):
        x = min(max(x, 0), w - 1); y = min(max(y, 0), h - 1); return im[y][x]
    out = []
    for oy in range(h * k):
        fy = (oy + 0.5) / k - 0.5; y0 = int(fy // 1); ty = fy - y0; row = []
        for ox in range(w * k):
            fx = (ox + 0.5) / k - 0.5; x0 = int(fx // 1); tx = fx - x0
            acc = [0.0] * 4
            for (dx, dy, wt) in ((0, 0, (1 - tx) * (1 - ty)), (1, 0, tx * (1 - ty)), (0, 1, (1 - tx) * ty), (1, 1, tx * ty)):
                p = at(x0 + dx, y0 + dy); a = p[3] / 255
                acc[0] += p[0] * a * wt; acc[1] += p[1] * a * wt; acc[2] += p[2] * a * wt; acc[3] += a * wt
            row.append(CLEAR if acc[3] <= 0 else (round(acc[0] / acc[3]), round(acc[1] / acc[3]), round(acc[2] / acc[3]), round(acc[3] * 255)))
        out.append(row)
    return out

def epx(im):
    """EPX / Scale2x: a pixel becomes four, a corner takes the neighbour's colour where two neighbours agree."""
    h, w = len(im), len(im[0])
    def at(x, y): return im[min(max(y, 0), h - 1)][min(max(x, 0), w - 1)]
    out = [[CLEAR] * (w * 2) for _ in range(h * 2)]
    for y in range(h):
        for x in range(w):
            p = at(x, y); a = at(x, y - 1); b = at(x + 1, y); c = at(x - 1, y); d = at(x, y + 1)
            e = [p, p, p, p]
            if c == a and c != d and a != b: e[0] = a
            if a == b and a != c and b != d: e[1] = b
            if d == c and d != b and c != a: e[2] = c
            if b == d and b != a and d != c: e[3] = d
            out[2 * y][2 * x], out[2 * y][2 * x + 1], out[2 * y + 1][2 * x], out[2 * y + 1][2 * x + 1] = e
    return out

def smooth_palette(im):
    """Bilinear x8 on premultiplied colour, then every pixel snapped to the nearest colour the source has and the
    edge cut at half alpha: smooth contours, flat colours (no new tints)."""
    big = bilinear(im, K)
    pal = sorted({p[:3] for row in im for p in row if p[3] > 127})
    def snap(p):
        return min(pal, key=lambda q: (q[0] - p[0]) ** 2 + (q[1] - p[1]) ** 2 + (q[2] - p[2]) ** 2)
    cache = {}
    out = []
    for row in big:
        r = []
        for p in row:
            if p[3] < 128: r.append(CLEAR); continue
            k = p[:3]
            if k not in cache: cache[k] = snap(p)
            r.append(cache[k] + (255,))
        out.append(r)
    return out

def over_grey(im, g=(70, 70, 80)):
    return [[(p[0] * p[3] // 255 + g[0] * (255 - p[3]) // 255, p[1] * p[3] // 255 + g[1] * (255 - p[3]) // 255, p[2] * p[3] // 255 + g[2] * (255 - p[3]) // 255, 255) for p in row] for row in im]

def sheet(images):
    h = max(len(i) for i in images); gap = 8
    w = sum(len(i[0]) for i in images) + gap * (len(images) + 1)
    out = [[(40, 40, 48, 255)] * w for _ in range(h + 2 * gap)]
    x = gap
    for im in images:
        for j, row in enumerate(im):
            out[gap + j][x:x + len(row)] = row
        x += len(im[0]) + gap
    return out

# the device's icons: OAM pieces of the lower engine (index.csv of `dump2d` in the device menu)
ICONS = {'guide': range(22, 32), 'memo': range(32, 38), 'save': range(38, 42), 'goods': range(42, 48)}
ARROW = [(1, 0), (1, 1)]  # engine 1, OAM 0 and 1 of the arrow's own pieces (x220, y170 / y186)

def main():
    src, out = sys.argv[1:3]
    os.makedirs(out, exist_ok=True)
    rows = list(csv.DictReader(open(os.path.join(src, 'index.csv'))))
    obj = {(int(r['engine']), int(r['layer_or_oam'])): (int(r['x']), int(r['y']), read_png(os.path.join(src, r['file']))) for r in rows if r['kind'] == 'obj'}
    sets = {n: [obj[(1, i)] for i in idx if (1, i) in obj] for n, idx in ICONS.items()}
    sets['arrow'] = [obj[(1, 0)], obj[(1, 1)]]
    for name, pieces in sets.items():
        im = compose(src, pieces)
        methods = [('nearest', nearest(im)), ('bilinear', bilinear(im)), ('epx', epx(epx(im))), ('smooth_palette', smooth_palette(im))]
        for m, big in methods:
            write_png(os.path.join(out, f'{name}_{m}.png'), big)
        write_png(os.path.join(out, f'{name}_original.png'), im)
        write_png(os.path.join(out, f'{name}_sheet.png'), sheet([over_grey(b) for _, b in methods]))
        print(name, f'{len(im[0])}x{len(im)}', 'ok')

if __name__ == '__main__':
    main()
