#!/usr/bin/env python3
"""Выгрузка картинок интерфейса чемодана Катарины из `DATA/NAKA/NAKA.BIN` (PS2, только чтение), этап 8е.7а.

    case.py DISC.iso OUTDIR [ЗАПИСИ]      PNG каждой картинки и OUTDIR/index.csv

ЗАПИСИ — номера через запятую (по умолчанию 15,16,19). Имя файла: ЗАПИСЬ_ИНДЕКС_WxH_ФОРМАТ.png.
В index.csv: файл, запись, индекс, ширина, высота, формат, палитра (цветов, 0 — без неё), dbp, dbw, прозрачных пикселей (%), вид (picture/palette; палитры идут строками без PNG).
"""
import csv
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import iso as isomod
import lf


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    disc = isomod.Iso(argv[0])
    wanted = [int(x) for x in argv[2].split(',')] if len(argv) > 2 else [15, 16, 19]
    e = disc.find('DATA/NAKA/NAKA.BIN')
    arch = lf.Archive(disc.read(e['lba'], e['size']))
    os.makedirs(argv[1], exist_ok=True)
    rows, skipped = [], []
    for ent in arch.entries:
        if ent.index not in wanted:
            continue
        for im in ent.images:
            if im.is_clut:
                rows.append([f'(палитра) {ent.index:03}_{im.index:03}', ent.index, im.index, im.w, im.h, im.name, im.w * im.h, im.dbp, im.dbw, 0, 'palette'])
                continue
            clut = lf.pair_clut(ent, im)
            rgba = lf.to_rgba(im, clut)
            if not rgba:
                skipped.append((ent.index, im.index, im.w, im.h, im.name))
                continue
            name = f'{ent.index:03}_{im.index:03}_{im.w}x{im.h}_{im.name}.png'
            lf.write_png(os.path.join(argv[1], name), im.w, im.h, rgba)
            clear = sum(1 for i in range(3, len(rgba), 4) if rgba[i] == 0) * 100 // max(1, im.w * im.h)
            rows.append([name, ent.index, im.index, im.w, im.h, im.name, len(clut) if clut else 0, im.dbp, im.dbw, clear, 'picture'])
    with open(os.path.join(argv[1], 'index.csv'), 'w', newline='') as f:
        w = csv.writer(f)
        w.writerow(['file', 'entry', 'index', 'w', 'h', 'format', 'palette', 'dbp', 'dbw', 'clear_pct', 'kind'])
        w.writerows(rows)
    print(f'строк в каталоге {len(rows)}, картинок {sum(1 for r in rows if r[-1] == 'picture')}, не открылось {len(skipped)}', skipped[:10])
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
