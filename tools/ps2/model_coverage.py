#!/usr/bin/env python3
"""Охват сцен и таблица «DS-модель → PS2-меш» (K-112).

    model_coverage.py --pairs k001-report.csv --catalog index_metrics.csv [--catalog ...] \
                      [--out-models models.csv] [--out-ds ds_only.csv] [--summary]

Вход:
  --pairs    отчёт K-001: `name` (ключ текстуры DS WxH_ф_данные_палитра.png), `source` (DATA/…/АРХИВ.BIN#запись.текстура).
  --catalog  `index_metrics.csv` из `check_models.py` (повторяется по числу каталогов): модель = (архив, запись),
             статус K-183 (ok/check/bad), текстуры `NNN_K.png:ШxВ`.
Связь DS↔PS2 идёт только через пары текстур: у DS-пачек в трассе GX есть ключ, у PS2-записи — индекс текстуры.

Решение по модели PS2:
  swap        статус ok и есть пара DS-ключей: подменять по ключу (K-111/K-128)
  blocked     есть пара, но модель красная/спорная (части не расставлены, K-199): ждёт разбора
  unpaired    меш есть, пар нет: привязать по геометрии/трассе или не брать
Текстуры DS без пары — «оставить DS» (вход для K-113, апскейл/перерисовка).
"""
import argparse
import collections
import csv
import re
import sys

TEX = re.compile(r'\d+_(\d+)\.png:(\d+)x(\d+)')


def read_pairs(path):
    """[(ключ DS, архив, запись, текстура)] для строк с источником; и все ключи DS."""
    paired, keys = [], []
    for r in csv.DictReader(open(path, newline='')):
        key = r['name'].rsplit('.', 1)[0]
        keys.append(key)
        src = r.get('source') or ''
        m = re.search(r'([A-Z0-9]+)\.BIN#(\d+)\.(\d+)$', src)
        if m:
            paired.append((key, m.group(1), int(m.group(2)), int(m.group(3))))
    return paired, keys


def read_models(paths):
    out = {}
    for p in paths:
        for r in csv.DictReader(open(p, newline='')):
            out[(r['архив'], int(r['запись']))] = r
    return out


def klass(m):
    """Составная (части, кости) или статичная: по числу «частей-двойников» K-183."""
    return 'составная' if int(m.get('m_stacked') or 0) >= 4 or int(m.get('m_parts') or 0) >= 4 else 'статичная'


def build(pairs, models):
    by_model = collections.defaultdict(list)
    for key, arch, entry, tex in pairs:
        by_model[(arch, entry)].append((key, tex))
    rows = []
    for (arch, entry), m in sorted(models.items(), key=lambda kv: (kv[0][0], kv[0][1])):
        tex = TEX.findall(m.get('картинки', ''))
        ds = by_model.get((arch, entry), [])
        status = m.get('m_status', '')
        if ds and status == 'ok':
            dec = 'swap'
        elif ds:
            dec = 'blocked'
        else:
            dec = 'unpaired'
        rows.append({
            'архив': arch, 'запись': entry, 'статус': status, 'класс': klass(m),
            'вершин': m['вершин'], 'треугольников': m['треугольников'],
            'текстур PS2': len(tex), 'пар DS': len({k for k, _ in ds}),
            'решение': dec, 'ключи DS': ' '.join(sorted({k for k, _ in ds})[:6]),
        })
    return rows


def textures_without_mesh(pairs, models):
    """Пары, у которых запись PS2 не дала меша: текстура есть, модели нет (интерфейс, спрайты, фоны)."""
    return sorted({(k, a, e) for k, a, e, _ in pairs if (a, e) not in models})


def ds_only(keys, pairs):
    have = {k for k, *_ in pairs}
    return [k for k in keys if k not in have]


def summary(rows, pairs, keys, models):
    by = collections.defaultdict(collections.Counter)
    for r in rows:
        by[r['архив']][r['решение']] += 1
        by[r['архив']]['всего'] += 1
    lines = ['%-10s %6s %6s %8s %9s' % ('архив', 'всего', 'swap', 'blocked', 'unpaired')]
    for a in sorted(by):
        c = by[a]
        lines.append('%-10s %6d %6d %8d %9d' % (a, c['всего'], c['swap'], c['blocked'], c['unpaired']))
    tot = collections.Counter(r['решение'] for r in rows)
    lines.append('моделей %d: swap %d, blocked %d, unpaired %d' % (len(rows), tot['swap'], tot['blocked'], tot['unpaired']))
    nomesh = textures_without_mesh(pairs, models)
    lines.append('DS-ключей %d: с парой PS2 %d, без пары (оставить DS) %d; пар в записях без меша: %d'
                 % (len(keys), len({k for k, *_ in pairs}), len(ds_only(keys, pairs)), len({k for k, _, _ in nomesh})))
    return '\n'.join(lines)


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument('--pairs', required=True)
    ap.add_argument('--catalog', action='append', required=True)
    ap.add_argument('--out-models')
    ap.add_argument('--out-ds')
    ap.add_argument('--summary', action='store_true')
    a = ap.parse_args(argv)
    pairs, keys = read_pairs(a.pairs)
    models = read_models(a.catalog)
    rows = build(pairs, models)
    if a.out_models:
        with open(a.out_models, 'w', newline='') as f:
            w = csv.DictWriter(f, fieldnames=list(rows[0]))
            w.writeheader()
            w.writerows(rows)
    if a.out_ds:
        with open(a.out_ds, 'w', newline='') as f:
            f.write('ключ DS\n')
            for k in ds_only(keys, pairs):
                f.write(k + '\n')
    print(summary(rows, pairs, keys, models))


if __name__ == '__main__':
    main(sys.argv[1:])
