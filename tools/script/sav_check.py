#!/usr/bin/env python3
"""K-170: проверка и пересчёт контрольной суммы слота .sav (DS, Flower, Sun and Rain).
Слот 0x3B00 байт; сумма — 16-битная с переносом (как у IP), инвертированная, по байтам слота [0x4C, 0x33BC),
хранится LE u16 в слоте+0x33BC (функция ARM9 0x020111E4, вызов из 0x02057A20). Заголовок 0x00–0x4B:
имя «Flower_Sun_Rain», +0x10 версия 0x160BC, +0x14 == +0x1C (счётчик записи).
Запуск: python3 -I tools/script/sav_check.py FILE [--fix OUT]"""
import struct, sys

SLOT, LO, HI = 0x3B00, 0x4C, 0x33BC


def checksum(slot: bytes) -> int:
    d = slot[LO:HI]
    s = sum(d[i] | d[i + 1] << 8 for i in range(0, len(d) - 1, 2)) + (d[-1] if len(d) & 1 else 0)
    while s >> 16:
        s = (s & 0xFFFF) + (s >> 16)
    return ~s & 0xFFFF


def report(slot: bytes) -> str:
    st = struct.unpack_from("<H", slot, HI)[0]
    ok = slot[:16] == b"Flower_Sun_Rain\0"
    c14, c1c = struct.unpack_from("<I", slot, 0x14)[0], struct.unpack_from("<I", slot, 0x1C)[0]
    return f"name={'ok' if ok else 'BAD'} counter {c14}/{c1c} {'ok' if c14 == c1c else 'MISMATCH'} sum stored {st:04X} computed {checksum(slot):04X} {'ok' if st == checksum(slot) else 'MISMATCH'}"


if __name__ == "__main__":
    data = bytearray(open(sys.argv[1], "rb").read())
    print(report(bytes(data[:SLOT])))
    if "--fix" in sys.argv:
        struct.pack_into("<H", data, HI, checksum(bytes(data[:SLOT])))
        open(sys.argv[sys.argv.index("--fix") + 1], "wb").write(data)
