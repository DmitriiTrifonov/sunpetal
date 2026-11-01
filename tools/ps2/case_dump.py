#!/usr/bin/env python3
"""Интерфейс чемодана на PS2: запись 16 архива NAKA.BIN (разведка 8е.7а2, только чтение).

Запись 16 — не модель, а набор готовых картинок: заголовок `u32 count`, потом `count × {u32 offset, u32 size}`
(подресурсы, у каждого DMA-тег и загрузка картинки GS), без вершин и UV. Кейс — предрисованный рендер 352×420 T8
(`#004` с журналом на дне, `#006` пустой); в WAT.BIN запись 17 — тот же кейс 284×384 под наклоном.

    case_dump.py NAKA.BIN OUTDIR     подресурсы, PNG картинок, таблица `table.csv`
"""
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lf  # noqa: E402


def main(argv):
    a = lf.Archive(open(argv[0], 'rb').read())
    out = argv[1]
    os.makedirs(out, exist_ok=True)
    e = a.entries[16]
    d = e.data
    n = struct.unpack_from('<I', d, 0)[0]
    rows = ['sub,offset,size,images']
    for i in range(n):
        off, size = struct.unpack_from('<II', d, 4 + 8 * i)
        sub = lf.Entry(i, 0, d[off:off + size])
        names = []
        for im in sub.images:
            if im.is_clut:
                continue
            rgba = lf.to_rgba(im, lf.pair_clut(sub, im))
            if rgba:
                nm = f'{i:03}_{im.w}x{im.h}_{im.name}.png'
                lf.write_png(os.path.join(out, nm), im.w, im.h, rgba)
                names.append(nm)
        rows.append(f'{i},{off:#x},{size:#x},{" ".join(names)}')
    open(os.path.join(out, 'table.csv'), 'w').write('\n'.join(rows) + '\n')
    print(n, 'подресурсов')


if __name__ == '__main__':
    main(sys.argv[1:])
