#!/usr/bin/env python3
"""Читатель сохранений PCSX2 (`.p2s`, только чтение).

Файл — zip, записи сжаты zstd (метод 93). Модуль zipfile такие не читает,
поэтому разбираем центральный каталог сами, распаковываем системным `zstd`.

    p2s.py STATE ls                    список записей
    p2s.py STATE cat NAME > file       содержимое записи
    p2s.py STATE extract DIR [NAME..]  записи в каталог (по умолчанию все)
"""
import os
import struct
import subprocess
import sys


def entries(path):
    d = open(path, "rb").read()
    i = d.rfind(b"PK\x05\x06")
    if i < 0:
        raise SystemExit("не zip")
    n, = struct.unpack_from("<H", d, i + 10)
    off, = struct.unpack_from("<I", d, i + 16)
    out = {}
    for _ in range(n):
        (_, _, _, _, method, _, _, _, csize, usize, nlen, elen, clen, _, _, _,
         lho) = struct.unpack_from("<IHHHHHHIIIHHHHHII", d, off)
        name = d[off + 46:off + 46 + nlen].decode()
        off += 46 + nlen + elen + clen
        nl, el = struct.unpack_from("<HH", d, lho + 26)
        start = lho + 30 + nl + el
        out[name] = (method, d[start:start + csize], usize)
    return out


def read(ents, name):
    method, raw, usize = ents[name]
    if method == 0:
        return raw
    if method == 93:
        r = subprocess.run(["zstd", "-d", "-c", "-q"], input=raw, capture_output=True, check=True)
        return r.stdout
    raise SystemExit(f"метод сжатия {method} не поддержан")


def main():
    if len(sys.argv) < 3:
        raise SystemExit(__doc__)
    ents = entries(sys.argv[1])
    cmd = sys.argv[2]
    if cmd == "ls":
        for k, (m, raw, u) in ents.items():
            print(f"{u:10d}  {k}")
    elif cmd == "cat":
        sys.stdout.buffer.write(read(ents, sys.argv[3]))
    elif cmd == "extract":
        os.makedirs(sys.argv[3], exist_ok=True)
        for k in sys.argv[4:] or ents:
            open(os.path.join(sys.argv[3], k), "wb").write(read(ents, k))
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
