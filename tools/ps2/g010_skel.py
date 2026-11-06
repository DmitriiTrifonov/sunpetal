#!/usr/bin/env python3
"""Скелет из записи `G010` PS2 (только чтение). Разведка K-153/K-154, см. docs/PS2-RECON.md.

    g010_skel.py ARCHIVE.BIN [INDEX]    скелеты записей G010 (или одной записи)

Узел — 0x140 байт, первый на смещении 0x100 записи:
  +0x40..0x100  три единичные матрицы 4x4 (место под матрицы рантайма)
  +0x104        u16 first_child+1, u16 next_sibling+1 (0xffff — нет)
  +0x10c        u32 id кости (0x065e00NN)
  +0x110        6 f32: смещение xyz (+ три малых числа, вероятно эйлеровы углы), дубль со +0x128
Таблица кончается там, где поля +0x104 перестают быть похожи на индексы.
"""
import struct, sys

NODE, FIRST = 0x140, 0x100


def records(d):
    n, = struct.unpack_from('<H', d, 2)
    off = 0x800
    for k in range(n):
        s, _ = struct.unpack_from('<II', d, 8 + 8 * k)
        yield k, d[off:off + s]
        off += (s + 0x7ff) // 0x800 * 0x800


def skeleton(r):
    nodes = []
    for i in range(200):
        o = FIRST + NODE * i + 0x100
        if o + 0x30 > len(r):
            break
        ch, sib = struct.unpack_from('<HH', r, o + 4)
        ident = struct.unpack_from('<16f', r, FIRST + NODE * i + 0x40)
        if ident[0] != 1.0 or ident[5] != 1.0:
            break
        nid, = struct.unpack_from('<I', r, o + 12)
        nodes.append({'i': i, 'child': ch - 1 if ch != 0xffff else None, 'sib': sib - 1 if sib != 0xffff else None,
                      'id': nid, 'v': struct.unpack_from('<6f', r, o + 16)})
    return nodes


if __name__ == '__main__':
    d = open(sys.argv[1], 'rb').read()
    want = int(sys.argv[2]) if len(sys.argv) > 2 else None
    for k, r in records(d):
        if r[:4] == b'G010' and want in (None, k):
            ns = skeleton(r)
            print(f'#{k}: {len(ns)} узлов')
            for n in ns:
                print(f"  {n['i']:2} child={n['child']} sib={n['sib']} id={n['id']:#x} " + ' '.join(f'{x:8.3f}' for x in n['v']))
