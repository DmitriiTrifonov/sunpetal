#!/usr/bin/env python3
"""Манифест подмены моделей PS2 → файл подмены рантайма (K-128, `SUNPETAL_MODEL_SWAP`).

    swapman.py MANIFEST.json OUT.swap [ИМЯ] [--watch]

Манифест (JSON):
    {"raw": "build/ps2-models/raw",
     "swaps": {"car": {
        "key":  "128x128_5_8def2870adbfb901_f387cdfc24eb54ac",   ключ текстуры DS (TexKey: размер_формат_данные_палитра)
        "size": 128,                                                 сторона текстуры в UV
        "hide": ["64x64_5_0430bc758dfac332_58a85f96628fe037"],      ключи, чьи полигоны только скрыть
        "ps2":  {"archive": "OP.BIN", "entry": 18},                  запись архива LF
        "orient": {"perm": [0,2,1], "signs": [-1,1,1], "scale": 0.04, "offset": [0.0002,-0.2,-0.6166],
                   "flipfront": true},                                оси: perm/знаки, масштаб, сдвиг; flipfront = поворот на 180°
        "color": 90,  "rev": false, "vflip": false }}}
Игра перечитывает файл подмены сама (раз в 30 кадров); --watch пересобирает его при правке манифеста.
Рантайм держит одну подмену: без ИМЕНИ берётся единственная в манифесте.
"""
import json
import math
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lf
import g101_verts as g

_cache = {}


def mesh(raw, archive, entry):
    k = (raw, archive, entry)
    if k not in _cache:
        a = lf.Archive(open(os.path.join(raw, archive), 'rb').read())
        for e in a.entries:
            if e.index == entry:
                _cache[k] = g.entry_mesh(bytes(e.data))[0]
                break
        else:
            raise SystemExit('нет записи %d в %s' % (entry, archive))
    return _cache[k]


def matrix(o):
    """(строки E по столбцам, сдвиг): q = v*E + shift."""
    perm = o.get('perm', [0, 1, 2])
    sg = list(o.get('signs', [1, 1, 1]))
    sc = o.get('scale', 1.0)
    if o.get('flipfront'):
        sg[0], sg[1] = -sg[0], -sg[1]
    E = [[0] * 3 for _ in range(3)]
    for i in range(3):
        E[perm[i]][i] = sg[i] * sc
    return E, o.get('offset', [0, 0, 0])


def build(sw, raw, base='.'):
    """Текст файла подмены по описанию одной подмены."""
    E, off = matrix(sw.get('orient', {}))
    out = ['key %s' % sw['key'], 'size %d' % sw.get('size', 128)]
    out += ['hide %s' % h for h in sw.get('hide', [])]
    out.append('e ' + ' '.join('%.6f' % E[j][i] for j in range(3) for i in range(3)) + ' ' + ' '.join('%.6f' % x for x in off))
    col = sw.get('color', 127)
    ps2 = sw['ps2']
    for t in mesh(os.path.join(base, raw), ps2['archive'], ps2['entry']):
        if not all(math.isfinite(x) and abs(x) < 1e4 for r in t for x in r[:3]):
            continue
        for k in ([0, 2, 1] if sw.get('rev') else [0, 1, 2]):
            r = t[k]
            u, v = r[12], r[13]
            if sw.get('vflip'):
                v = 1 - v
            out.append('v %.4f %.4f %.4f %.5f %.5f %d %d %d' % (r[0], r[1], r[2], u, v, col, col, col))
    return '\n'.join(out) + '\n'


def pick(man, name):
    sws = man.get('swaps') or {}
    if name is None:
        if len(sws) != 1:
            raise SystemExit('в манифесте %d подмен, назовите одну: %s' % (len(sws), ', '.join(sws)))
        name = next(iter(sws))
    if name not in sws:
        raise SystemExit('нет подмены %s' % name)
    return sws[name]


def run(manifest, out, name=None):
    man = json.load(open(manifest))
    text = build(pick(man, name), man.get('raw', 'build/ps2-models/raw'))
    tmp = out + '.tmp'
    open(tmp, 'w').write(text)
    os.replace(tmp, out)   # игра не прочтёт половину файла
    return text.count('\nv ') 


def main(a):
    watch = '--watch' in a
    a = [x for x in a if x != '--watch']
    if len(a) < 2:
        print(__doc__)
        return 2
    name = a[2] if len(a) > 2 else None
    last = None
    while True:
        m = os.path.getmtime(a[0])
        if m != last:
            last = m
            try:
                print('треугольников:', run(a[0], a[1], name) // 3, flush=True)
            except (SystemExit, ValueError, KeyError, OSError) as e:
                print('ошибка манифеста:', e, flush=True)
        if not watch:
            return 0
        time.sleep(1)


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
