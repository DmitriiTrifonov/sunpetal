#!/usr/bin/env python3
"""Разборщик текстовой трассы вершинных пачек DS-движка GX и каталог «DS-объектов» по ключу текстуры.

Трасса — строки трёх видов (см. `tools/ps2/testdata/gx_trace_sample.txt`):

    f=3000 vb=6112 b=12 prim=1 n=4 min=[...] max=[...] tex=0x... pos=[16 чисел]   пачка
    K f=3000 tex=0x... pal=0x... key=ШxВ_формат_хэшДанных_хэшПалитры             ключ картинки
    V f=3000 b=12 x y z                                                        вершина (необязательно)

Пачка не несёт `pal`, поэтому внутри кадра пачка сопоставляется ключу по `tex`; если на один
`tex` в кадре приходится несколько ключей, объект помечается неоднозначным. Пачки без `K`-строки
группируются по `tex` с пометкой «ключ неизвестен».

    dsobjects.py TRACE [--csv OUT.csv] [--json OUT.json]

Читает построчно (файлы бывают сотни МБ), пропускает оборванные в конце строки. Выводит таблицу
в stdout (по убыванию числа вершин) и, по флагам, CSV/JSON. Для тестов доступны `parse_trace(lines)`
и `build_catalog(records)`.
"""
import argparse
import csv
import json
import re
import sys

# Фиксированная точка 1.3.12: 4096 == 1.0.
UNIT = 4096.0

_BATCH_RE = re.compile(
    r'^f=(\d+)\s+vb=(\d+)\s+b=(\d+)\s+prim=(\d+)\s+n=(\d+)\s+'
    r'min=\[([^\]]*)\]\s+max=\[([^\]]*)\]\s+'
    r'tex=(0x[0-9a-fA-F]+|\d+)\s+pos=\[([^\]]*)\]$'
)
_KEY_RE = re.compile(
    r'^K\s+f=(\d+)\s+tex=(0x[0-9a-fA-F]+|\d+)\s+pal=(0x[0-9a-fA-F]+|\d+)\s+key=(\S+)$'
)
_VERTEX_RE = re.compile(r'^V\s+f=(\d+)\s+b=(\d+)\s+(-?\d+)\s+(-?\d+)\s+(-?\d+)$')


def _ints(text, count):
    """Разбирает `text` (через запятую) ровно в `count` целых; иначе None."""
    parts = text.split(',')
    if len(parts) != count:
        return None
    try:
        return tuple(int(p.strip()) for p in parts)
    except ValueError:
        return None


def _units(value):
    """Целое 1.3.12 -> float в единицах 1.0, округлённый до 3 знаков (без -0.0)."""
    x = round(value / UNIT, 3)
    return 0.0 if x == 0 else x


def parse_trace(lines):
    """Генератор записей трассы. `lines` — итератор строк (файл или список).

    Пропускает пустые и оборванные строки. Запись — словарь с полем `type`:

        batch:  f, vb, b, prim, n, min=(x,y,z), max=(x,y,z), tex=int, pos=(16 int)
        key:    f, tex=int, pal=int, key=str
        vertex: f, b, x, y, z
    """
    for line in lines:
        line = line.strip()
        if not line:
            continue
        first = line[0]
        if first == 'f':
            m = _BATCH_RE.match(line)
            if not m:
                continue
            mn = _ints(m.group(6), 3)
            mx = _ints(m.group(7), 3)
            pos = _ints(m.group(9), 16)
            if mn is None or mx is None or pos is None:
                continue
            yield {'type': 'batch', 'f': int(m.group(1)), 'vb': int(m.group(2)),
                   'b': int(m.group(3)), 'prim': int(m.group(4)), 'n': int(m.group(5)),
                   'min': mn, 'max': mx, 'tex': int(m.group(8), 0), 'pos': pos}
        elif first == 'K':
            m = _KEY_RE.match(line)
            if not m:
                continue
            yield {'type': 'key', 'f': int(m.group(1)), 'tex': int(m.group(2), 0),
                   'pal': int(m.group(3), 0), 'key': m.group(4)}
        elif first == 'V':
            m = _VERTEX_RE.match(line)
            if not m:
                continue
            yield {'type': 'vertex', 'f': int(m.group(1)), 'b': int(m.group(2)),
                   'x': int(m.group(3)), 'y': int(m.group(4)), 'z': int(m.group(5))}


def _classify(matrices, vertices):
    if 1 <= matrices <= 4 and 800 <= vertices <= 1300:
        return 'car-like'
    if matrices >= 8:
        return 'character-like'
    return 'other'


def _merge_group(fd, data):
    """Сливает агрегат пачек (f, tex) в данные кадра объекта."""
    batches, vertices, matrices = data
    fd[0] += batches
    fd[1] += vertices
    for mat, agg in matrices.items():
        cur = fd[2].get(mat)
        if cur is None:
            fd[2][mat] = list(agg)
        else:
            cur[0] += agg[0]
            cur[1] = min(cur[1], agg[1])
            cur[2] = max(cur[2], agg[2])
            cur[3] = min(cur[3], agg[3])
            cur[4] = max(cur[4], agg[4])
            cur[5] = min(cur[5], agg[5])
            cur[6] = max(cur[6], agg[6])


def build_catalog(records):
    """Список объектов-словарей (по убыванию числа вершин), сгруппированных по ключу текстуры.

    Поля объекта: `key` (строка или None), `tex` (список значений), `key_unknown`, `ambiguous`,
    `frames`, `frame_min`, `frame_max`, `frame` (типичный кадр), `batches`, `vertices`, `matrices`
    (число различных матриц), `matrix_stats` (на матрицу: vertices, min, max в единицах 1.0),
    `size` ([dx, dy, dz] объединением по матрицам), `class`.
    """
    keys_by_ft = {}   # (f, tex) -> список ключей (без дублей, в порядке появления)
    frame_tex = {}    # (f, tex) -> [пачек, вершин, {матрица: [вершин, mnx, mxx, mny, mxy, mnz, mxz]}]

    for r in records:
        t = r['type']
        if t == 'key':
            ft = (r['f'], r['tex'])
            lst = keys_by_ft.setdefault(ft, [])
            if r['key'] not in lst:
                lst.append(r['key'])
        elif t == 'batch':
            ft = (r['f'], r['tex'])
            data = frame_tex.get(ft)
            if data is None:
                data = [0, 0, {}]
                frame_tex[ft] = data
            data[0] += 1
            data[1] += r['n']
            mat = r['pos']
            agg = data[2].get(mat)
            mnx, mny, mnz = r['min']
            mxx, mxy, mxz = r['max']
            if agg is None:
                data[2][mat] = [r['n'], mnx, mxx, mny, mxy, mnz, mxz]
            else:
                agg[0] += r['n']
                agg[1] = min(agg[1], mnx)
                agg[2] = max(agg[2], mxx)
                agg[3] = min(agg[3], mny)
                agg[4] = max(agg[4], mxy)
                agg[5] = min(agg[5], mnz)
                agg[6] = max(agg[6], mxz)

    objects = {}

    def _object(oid):
        obj = objects.get(oid)
        if obj is None:
            obj = {'key': None, 'tex': set(), 'key_unknown': False, 'ambiguous': False,
                   'frames': set(), 'frame_data': {}}
            objects[oid] = obj
        return obj

    # Ключи с K-строкой, даже без пачек (кадры, где ключ встречался).
    for (f, tex), keys in keys_by_ft.items():
        for key in keys:
            obj = _object(('key', key))
            obj['key'] = key
            obj['tex'].add(tex)
            obj['frames'].add(f)
            if len(keys) > 1:
                obj['ambiguous'] = True

    # Пачки: сопоставляем (f, tex) ключам; без K-строки — объект по tex.
    for (f, tex), data in frame_tex.items():
        keys = keys_by_ft.get((f, tex))
        if not keys:
            obj = _object(('tex', tex))
            obj['key_unknown'] = True
            obj['tex'].add(tex)
            obj['frames'].add(f)
            targets = [obj]
        else:
            targets = [_object(('key', k)) for k in keys]
            if len(keys) > 1:
                for obj in targets:
                    obj['ambiguous'] = True
        for obj in targets:
            fd = obj['frame_data'].get(f)
            if fd is None:
                fd = [0, 0, {}]
                obj['frame_data'][f] = fd
            _merge_group(fd, data)

    result = []
    for obj in objects.values():
        frames = sorted(obj['frames'])
        if obj['frame_data']:
            frame = max(obj['frame_data'], key=lambda f: obj['frame_data'][f][1])
            fd = obj['frame_data'][frame]
        else:
            frame = frames[0] if frames else None
            fd = [0, 0, {}]
        batches, vertices, matrices = fd

        matrix_stats = []
        for mat, agg in matrices.items():
            matrix_stats.append({
                'matrix': mat,
                'vertices': agg[0],
                'min': [_units(agg[1]), _units(agg[3]), _units(agg[5])],
                'max': [_units(agg[2]), _units(agg[4]), _units(agg[6])],
            })
        matrix_stats.sort(key=lambda m: (-m['vertices'], m['matrix']))

        if matrices:
            overall_min = [min(a[1] for a in matrices.values()),
                           min(a[3] for a in matrices.values()),
                           min(a[5] for a in matrices.values())]
            overall_max = [max(a[2] for a in matrices.values()),
                           max(a[4] for a in matrices.values()),
                           max(a[6] for a in matrices.values())]
            size = [_units(overall_max[i] - overall_min[i]) for i in range(3)]
        else:
            size = [0.0, 0.0, 0.0]

        result.append({
            'key': obj['key'],
            'tex': sorted(obj['tex']),
            'key_unknown': obj['key_unknown'],
            'ambiguous': obj['ambiguous'],
            'frames': len(frames),
            'frame_min': frames[0] if frames else None,
            'frame_max': frames[-1] if frames else None,
            'frame': frame,
            'batches': batches,
            'vertices': vertices,
            'matrices': len(matrix_stats),
            'matrix_stats': matrix_stats,
            'size': size,
            'class': _classify(len(matrix_stats), vertices),
        })

    result.sort(key=lambda e: (-e['vertices'], e['key'] or ''))
    return result


def _fmt_matrix(mat):
    return '[' + ', '.join(str(x) for x in mat) + ']'


def _fmt_vec(v):
    return '[' + ', '.join(str(x) for x in v) + ']'


def format_catalog(catalog):
    """Человекочитаемая таблица объектов (текст)."""
    out = []
    for e in catalog:
        key = e['key'] if e['key'] is not None else '<tex %s>' % ','.join(hex(t) for t in e['tex'])
        flags = []
        if e['key_unknown']:
            flags.append('key unknown')
        if e['ambiguous']:
            flags.append('ambiguous')
        suffix = ('  [' + ', '.join(flags) + ']') if flags else ''
        rng = f"{e['frame_min']}..{e['frame_max']}" if e['frame_min'] is not None else '-'
        out.append(f'{key}{suffix}')
        out.append(f"  tex: {', '.join(hex(t) for t in e['tex'])}")
        out.append(f'  frames: {e["frames"]} [{rng}]')
        out.append(f"  typical f={e['frame']}: {e['batches']} batches, {e['vertices']} vertices, "
                   f"{e['matrices']} matrices")
        for m in e['matrix_stats']:
            out.append(f"    {_fmt_matrix(m['matrix'])}: {m['vertices']} vertices, "
                       f"min={_fmt_vec(m['min'])} max={_fmt_vec(m['max'])}")
        out.append(f"  size: dx={e['size'][0]} dy={e['size'][1]} dz={e['size'][2]}")
        out.append(f"  class: {e['class']}")
        out.append('')
    return '\n'.join(out)


_CSV_FIELDS = ['key', 'texs', 'key_unknown', 'ambiguous', 'frames', 'frame_min', 'frame_max',
               'frame', 'batches', 'vertices', 'matrices', 'class', 'dx', 'dy', 'dz']


def _csv_row(e):
    return {
        'key': e['key'] or '',
        'texs': ','.join(hex(t) for t in e['tex']),
        'key_unknown': e['key_unknown'],
        'ambiguous': e['ambiguous'],
        'frames': e['frames'],
        'frame_min': e['frame_min'],
        'frame_max': e['frame_max'],
        'frame': e['frame'],
        'batches': e['batches'],
        'vertices': e['vertices'],
        'matrices': e['matrices'],
        'class': e['class'],
        'dx': e['size'][0],
        'dy': e['size'][1],
        'dz': e['size'][2],
    }


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('trace', help='текстовая трасса GX')
    parser.add_argument('--csv', metavar='OUT.csv', help='писать CSV')
    parser.add_argument('--json', metavar='OUT.json', help='писать JSON')
    args = parser.parse_args(argv)

    with open(args.trace, 'r', encoding='utf-8', errors='replace') as f:
        catalog = build_catalog(parse_trace(f))

    print(format_catalog(catalog).rstrip('\n'))

    if args.csv:
        with open(args.csv, 'w', newline='', encoding='utf-8') as f:
            w = csv.DictWriter(f, fieldnames=_CSV_FIELDS)
            w.writeheader()
            for e in catalog:
                w.writerow(_csv_row(e))

    if args.json:
        with open(args.json, 'w', encoding='utf-8') as f:
            json.dump(catalog, f, indent=2, ensure_ascii=False)

    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]) or 0)
