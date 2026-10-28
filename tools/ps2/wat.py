#!/usr/bin/env python3
"""Читатель `DATA/WAT/WAT.BIN` (PS2-версия, только чтение).

Формат: обычный архив `LF` (см. lf.py), 57 записей. Типы записей по содержимому:
  - картинка-загрузка GS (DMA-тег + GIF A+D + IMAGE): 512x256 T8 (записи 3..10, содержимое
    занимает левые ~256 пикселей: виньетированные кадры мест), 320x240 T8 (11), 256x256 CT32/CT16,
    284x384 T8 (17), шрифты T4 (16, 24, 28..54: 192..560x32), числа и заставки (55, 56);
  - сырые данные без заголовка (0 и 21, 23: 0x40000/0x8bf0/0x10350 байт), не картинки GS;
  - служебные (19, 20, 22: сигнатуры `G010`, `pBAV`).

    wat.py WAT.BIN list              записи и картинки (размер, формат)
    wat.py WAT.BIN png OUTDIR        PNG каждой картинки: ЗАПИСЬ_ИНДЕКС_WxH_ФОРМАТ.png
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lf


def main(argv):
    if len(argv) < 2 or argv[1] not in ('list', 'png') or (argv[1] == 'png' and len(argv) < 3):
        print(__doc__)
        return 2
    arch = lf.Archive(open(argv[0], 'rb').read())
    for e in arch.entries:
        imgs = [im for im in e.images if not im.is_clut]
        kind = 'картинки: ' + ', '.join(f'{im.w}x{im.h} {im.name}' for im in imgs) if imgs \
            else f'не картинка GS, {len(e.data):#x} байт, голова {e.data[:4].hex()}'
        if argv[1] == 'list':
            print(f'[{e.index:2}] id={e.id:#x} {kind}')
        else:
            for im in imgs:
                rgba = lf.to_rgba(im, lf.pair_clut(e, im))
                if rgba:
                    os.makedirs(argv[2], exist_ok=True)
                    lf.write_png(os.path.join(argv[2], f'{e.index:03}_{im.index}_{im.w}x{im.h}_{im.name}.png'),
                                 im.w, im.h, rgba)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
