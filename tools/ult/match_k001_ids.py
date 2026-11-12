#!/usr/bin/env python3
"""Номера текстур из отчёта K-001 -> id картинок выгрузки `sunpetal-ee` (ULTIMATE-IMPORT.md, У4.1).

    match_k001_ids.py REPORT.csv DS_DUMP_DIR EXPORT_DIR [--model ID] [--source-prefix DATA/EVENT/R00.BIN#1]

Отчёт K-001 (`tools/ps2/mkpack.py`) называет источник `АРХИВ.BIN#запись.N` — номер картинки внутри
записи по нашему старому разбору GS-загрузок, к нумерации выгрузки он отношения не имеет
(проверено: у парных текстур номера не совпадают со слотами `textures.tsv`). Этот инструмент
устанавливает соответствие по содержимому: для каждой строки отчёта, чья DS-текстура лежит в
DS_DUMP_DIR, среди картинок выгрузки с размером «DS в k раз» (k = 1, 2, 4, 8) ищется лучшая по
средней ошибке цвета по непрозрачным пикселам DS. Печатает: ключ DS, источник, id, ошибку и
запас до второго места (малый запас — спорная пара, смотреть глазами). С --model дополнительно
проверяет, что id входит в материалы модели.

Только чтение; ничего никуда не пишет.
"""
import csv
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'ps2'))
import lf  # noqa: E402
from mkpack import shrink, mae  # noqa: E402


def load_tblocks(export):
    """textures.tsv -> {(width, height): [id, ...]} для kind == tblock."""
    by_size = {}
    with open(os.path.join(export, 'textures.tsv')) as f:
        for row in csv.reader(f, delimiter='\t'):
            if not row or row[0] == 'id' or row[1] != 'tblock':
                continue
            by_size.setdefault((int(row[2]), int(row[3])), []).append(row[0])
    return by_size


def main(argv):
    if len(argv) < 3:
        raise SystemExit(__doc__)
    report, dump, export = argv[0], argv[1], argv[2]
    model = argv[argv.index('--model') + 1] if '--model' in argv else None
    model_only = '--model-only' in argv
    prefix = argv[argv.index('--source-prefix') + 1] if '--source-prefix' in argv else None
    names = {n for n in os.listdir(dump) if n.endswith('.png')}
    by_size = load_tblocks(export)
    mat_tex = set()
    if model:
        import json
        j = json.load(open(os.path.join(export, 'models', model + '.json')))
        mat_tex = {m['texture'] for m in j['materials'] if m.get('texture')}
        if model_only:
            by_size = {k: [t for t in v if t in mat_tex] for k, v in by_size.items()}
    cache = {}

    def rgba(tid):
        if tid not in cache:
            cache[tid] = lf.read_png(os.path.join(export, 'textures', tid + '.png'))
        return cache[tid]

    rows = []
    with open(report) as f:
        for name, src, *_ in csv.reader(f):
            if name == 'name' or name not in names or not src:
                continue
            if prefix and not src.startswith(prefix):
                continue
            w, h = (int(v) for v in name.split('_', 1)[0].split('x'))
            _, _, ds = lf.read_png(os.path.join(dump, name))
            scored = []
            for k in (1, 2, 4, 8):
                for tid in by_size.get((w * k, h * k), []):
                    pw, ph, prgba = rgba(tid)
                    if (pw, ph) == (w * k, h * k):
                        scored.append((mae(ds, shrink(pw, ph, prgba, k)), tid))
            best = sorted(scored)
            if not best:
                rows.append((name, src, '-', 'нет кандидатов', ''))
                continue
            (e1, t1), (e2, t2) = best[0], best[1] if len(best) > 1 else (float('inf'), '-')
            in_model = '' if not model else ('в модели' if t1 in mat_tex else 'НЕТ В МОДЕЛИ')
            rows.append((name, src, t1, f'{e1:.2f}', f'+{e2 - e1:.2f} до {t2}', in_model))
    for r in sorted(rows, key=lambda r: r[1]):
        print('\t'.join(r))
    print(f'{len(rows)} строк, запас < 2.0 или ошибка > 12 — смотреть глазами', file=sys.stderr)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
