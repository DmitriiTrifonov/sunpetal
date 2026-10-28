#!/usr/bin/env python3
"""Читатель `YOSHI.BIN` с PS2-диска (только чтение).

`YOSHI.BIN` относится к модулю карты памяти и меню сохранения PS2-версии
(в программе: каталог исходников Yoshi, `Memcard.cpp`, `Title.cpp`, `Decode.cpp`).

Формат (по разведке, см. docs/PS2-RECON.md, раздел «YOSHI.BIN и GDB.BIN»):

    архив `LF` (см. lf.py): 'LF', count, version, таблица {size, id}, данные с 0x800,
    выравнивание 0x800.
    Запись:
        u32  размер после распаковки
        u32  размер сжатых данных (= размер записи - 8)
        ...  поток BPE (Byte Pair Encoding, Gage 1994), блоками

    Блок BPE:
        таблица пар: счётчик c; если c > 127, пропустить (c - 127) символов
                     (они кодируют сами себя); иначе (c + 1) символов подряд:
                     left[k] (1 байт), и если left[k] != k, то right[k] (1 байт);
                     повторять, пока не заполнятся 256 символов
        u16  (big-endian) размер сжатого блока, затем сам блок
    Распаковка блока: каждый байт кладётся на стек; пока стек не пуст: снять x;
    если left[x] == x вывести x, иначе положить right[x], затем left[x].
    Размер блока 5000 байт до сжатия.

Распакованная запись начинается с нескольких таблиц (смещения), дальше идёт та же цепочка
GIF-пакетов (загрузки картинок в GS), что и в `LF` (lf.py).

    yoshi.py YOSHI.BIN                 список записей (размеры, картинки)
    yoshi.py YOSHI.BIN dump N OUT      распакованная запись N в файл
    yoshi.py YOSHI.BIN png OUTDIR      PNG всех картинок из распакованных записей
"""
import importlib.util
import os
import struct
import sys


def _load_lf():
    spec = importlib.util.spec_from_file_location(
        'lf', os.path.join(os.path.dirname(os.path.abspath(__file__)), 'lf.py'))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def bpe_decode(src, want=None):
    """Поток BPE (блоки) -> байты. Останавливается на `want` байтах или конце входа."""
    out = bytearray()
    p = 0
    while p < len(src) and (want is None or len(out) < want):
        left = list(range(256))
        right = [0] * 256
        c = 0
        while c < 256:
            count = src[p]
            p += 1
            if count > 127:
                c += count - 127
                count = 0
                if c >= 256:
                    break
            for _ in range(count + 1):
                left[c] = src[p]
                p += 1
                if c != left[c]:
                    right[c] = src[p]
                    p += 1
                c += 1
        size = src[p] << 8 | src[p + 1]
        p += 2
        for b in src[p:p + size]:
            st = [b]
            while st:
                x = st.pop()
                if left[x] == x:
                    out.append(x)
                else:
                    st.append(right[x])
                    st.append(left[x])
        p += size
    return bytes(out)


def entry_bytes(lf, raw):
    """Список (id, распакованные данные, размер сжатых) для каждой записи архива."""
    a = lf.Archive(raw)
    res = []
    for e in a.entries:
        real, comp = struct.unpack_from('<II', e.data, 0)
        data = bpe_decode(e.data[8:8 + comp], real)
        res.append((e.id, data, comp, real))
    return res


def main(argv):
    if not argv:
        print(__doc__)
        return
    lf = _load_lf()
    raw = open(argv[0], 'rb').read()
    ents = entry_bytes(lf, raw)
    cmd = argv[1] if len(argv) > 1 else 'ls'
    if cmd == 'ls':
        for i, (ident, data, comp, real) in enumerate(ents):
            e = lf.Entry(i, ident, data)
            ok = 'ok' if len(data) == real else f'ОШИБКА {len(data)}'
            print(f'[{i}] id={ident:#x} сжато {comp}, распаковано {real} ({ok}), картинок {len(e.images)}')
            for im in e.images:
                print(f'      #{im.index} {im.w}x{im.h} {im.name} dbp={im.dbp:#x} dbw={im.dbw}'
                      f'{" (палитра)" if im.is_clut else ""}')
    elif cmd == 'dump':
        open(argv[3], 'wb').write(ents[int(argv[2])][1])
    elif cmd == 'png':
        out = argv[2]
        os.makedirs(out, exist_ok=True)
        for i, (ident, data, comp, real) in enumerate(ents):
            e = lf.Entry(i, ident, data)
            for im in e.images:
                if im.is_clut:
                    continue
                rgba = lf.to_rgba(im, lf.pair_clut(e, im))
                if rgba:
                    lf.write_png(os.path.join(out, f'{i:03}_{im.index}_{im.w}x{im.h}_{im.name}.png'),
                                 im.w, im.h, rgba)
    else:
        print(__doc__)


if __name__ == '__main__':
    main(sys.argv[1:])
