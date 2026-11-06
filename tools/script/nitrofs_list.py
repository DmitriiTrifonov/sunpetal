#!/usr/bin/env python3
"""Чтение NitroFS образа .nds (только чтение): файлы по путям.

    nitrofs_list.py ROM.nds --find СУБСТРОКА     список путей и размеров
    nitrofs_list.py ROM.nds --cat ПУТЬ OUT       выписать файл
    nitrofs_list.py ROM.nds --compare DATA_DIR   сравнить все файлы с распаковкой в папке (путь = относительный путь)
"""
import hashlib
import os
import struct
import sys


class Fs:
    def __init__(self, path):
        self.b = open(path, "rb").read()
        fnt, fnts, fat, fats = struct.unpack_from("<IIII", self.b, 0x40)
        self.fnt, self.fat = fnt, fat
        self.files = {}
        self._walk(0xF000, "")

    def _fat(self, i):
        return struct.unpack_from("<II", self.b, self.fat + 8 * i)

    def _walk(self, dirid, prefix):
        base = self.fnt + 8 * (dirid & 0xFFF)
        off, first, _ = struct.unpack_from("<IHH", self.b, base)
        p = self.fnt + off
        fid = first
        while True:
            t = self.b[p]
            p += 1
            if t == 0:
                break
            n = t & 0x7F
            name = self.b[p:p + n].decode("latin1")
            p += n
            if t & 0x80:
                sub = struct.unpack_from("<H", self.b, p)[0]
                p += 2
                self._walk(sub, prefix + name + "/")
            else:
                s, e = self._fat(fid)
                self.files[prefix + name] = self.b[s:e]
                fid += 1


def main():
    fs = Fs(sys.argv[1])
    a = sys.argv[2:]
    if a[0] == "--find":
        for k, v in sorted(fs.files.items()):
            if a[1] in k:
                print(k, len(v))
    elif a[0] == "--cat":
        open(a[2], "wb").write(fs.files[a[1]])
    elif a[0] == "--compare":
        same = diff = miss = 0
        diffs = []
        for k, v in fs.files.items():
            f = os.path.join(a[1], k)
            if not os.path.isfile(f):
                miss += 1
                continue
            if open(f, "rb").read() == v:
                same += 1
            else:
                diff += 1
                diffs.append(k)
        print(f"в ROM {len(fs.files)}, совпало {same}, отличается {diff}, нет в папке {miss}")
        for k in diffs[:10]:
            print("  отличается:", k)


main()
