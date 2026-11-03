#!/usr/bin/env python3
"""Модели PS2 → файлы `.spm` для секции MODL (K-110): `mkmodels.py MANIFEST.json OUTDIR [ИМЯ ...]`.

Манифест — тот же, что у `swapman.py` (`swaps`: запись PS2, ключ DS, `orient`). Для каждой подмены пишется
`OUTDIR/ИМЯ.spm` (формат — `crates/sunpetal-data/src/models.rs`): вершины без повторов, позиции уже в
единицах DS-объекта (ориентация, масштаб и сдвиг из манифеста запечены), нормаль повёрнута той же матрицей
без сдвига, `rev` меняет обход. Дальше: `sunpetal-pack-models --dir OUTDIR --out sunpetal.data [--merge ...]`.
Читает архив PS2 из `raw` манифеста (только чтение); ничего из него в репозиторий не кладётся.
"""
import json
import math
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import swapman


def parse_key(s):
    """'128x128_5_{data}_{pal}' → (data, pal, fmt, w, h)"""
    size, fmt, data, pal = s.split('_')
    w, h = size.split('x')
    return int(data, 16), int(pal, 16), int(fmt), int(w), int(h)


def convert(sw, raw):
    """-> (вершины [(x,y,z,nx,ny,nz,u,v)], индексы)"""
    E, off = swapman.matrix(sw.get('orient', {}))

    def mul(p, shift):
        return [sum(p[j] * E[j][i] for j in range(3)) + (off[i] if shift else 0) for i in range(3)]
    verts, index, idx = [], {}, []
    for t in swapman.mesh(raw, sw['ps2']['archive'], sw['ps2']['entry']):
        if not all(math.isfinite(x) and abs(x) < 1e4 for r in t for x in r[:3]):
            continue
        ids = []
        for k in ([0, 2, 1] if sw.get('rev') else [0, 1, 2]):
            r = t[k]
            u, v = r[12], r[13]
            if sw.get('vflip'):
                v = 1 - v
            p = mul(r[0:3], True)
            n = mul(r[4:7], False)
            ln = math.sqrt(sum(x * x for x in n)) or 1.0
            row = tuple(round(x, 5) for x in p + [x / ln for x in n] + [u, v])
            if row not in index:
                index[row] = len(verts)
                verts.append(row)
            ids.append(index[row])
        idx += ids
    return verts, idx


def encode(key, verts, idx):
    o = b'SPM1'
    if key:
        o += b'\1' + struct.pack('<QQBHH', *key)
    else:
        o += b'\0'
    o += struct.pack('<II', len(verts), len(idx))
    for v in verts:
        o += struct.pack('<8f', *v)
    return o + struct.pack('<%dI' % len(idx), *idx)


def main(a):
    if len(a) < 2:
        print(__doc__)
        return 2
    man = json.load(open(a[0]))
    raw = man.get('raw', 'build/ps2-models/raw')
    os.makedirs(a[1], exist_ok=True)
    for name, sw in man.get('swaps', {}).items():
        if a[2:] and name not in a[2:]:
            continue
        verts, idx = convert(sw, raw)
        open(os.path.join(a[1], name + '.spm'), 'wb').write(encode(parse_key(sw['key']) if 'key' in sw else None, verts, idx))
        print('%s: вершин %d, треугольников %d' % (name, len(verts), len(idx) // 3))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
