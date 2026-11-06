#!/usr/bin/env python3
"""K-161: карта «код команды сценария → обработчик ARM9» из таблицы переходов шага сценария.

    dispatch_map.py ROM generated/recomp/p_arm9_012.c [OUT.csv]

Шаг сценария (ARM9 `02045BC4`, Thumb) делает `switch` по коду команды: `cmp r0,#0x84`, таблица полуслов
со знаком по `02045BEC`, цель = (`02045BEE` + значение). Каждая ветка вызывает обработчик (`bl`); коды
соответствуют порядку имён в таблице ARM9 `0206556C` (код 0 — `<R>`, код 1 — `<TEXT>`, …).
Читает ROM и сгенерированный C (оба не в git, только чтение).
"""
import re
import struct
import sys

BASE = 0x02000000
TABLE = 0x02045BEC
NAMES = 0x02065568  # код 0 — <R>, дальше теги
COUNT = 0x85


def main():
    rom = open(sys.argv[1], "rb").read()
    a9 = rom[0x4000:0x4000 + 0x6E258]
    src = open(sys.argv[2], encoding="utf-8", errors="replace").read().split("\n")
    # метки блоков в функции шага: L_xxxxxxxx → список строк до следующей метки
    start = next(i for i, l in enumerate(src) if l.startswith("void p_arm9_t_02045BC4("))
    blocks, cur = {}, None
    for l in src[start + 1:]:
        if l.startswith("}"):
            break
        m = re.match(r"L_([0-9A-F]{8}):", l)
        if m:
            cur = int(m.group(1), 16)
            blocks[cur] = []
        elif cur is not None:
            blocks[cur].append(l)

    def first_bl(addr, seen=()):
        for l in blocks.get(addr, []):
            m = re.search(r"//\s*[0-9A-F]{8} bl 0x([0-9A-F]{8})", l)
            if m:
                return int(m.group(1), 16)
            for m in re.finditer(r"goto L_([0-9A-F]{8});", l):
                t = int(m.group(1), 16)
                if t not in seen and t != 0x02046168 and any("bl 0x" in x for x in blocks.get(t, [])):
                    return first_bl(t, seen + (addr,))
        return None

    def name(code):
        p = struct.unpack_from("<I", a9, NAMES - BASE + 4 * code)[0]
        o = p - BASE
        e = o
        while a9[e:e + 2] != b"\0\0":
            e += 2
        return a9[o:e].decode("utf-16le")

    rows = []
    for code in range(COUNT):
        h = struct.unpack_from("<h", a9, TABLE - BASE + 2 * code)[0]
        target = (0x02045BEE + h) & ~1
        h1 = None if code == 0 else first_bl(target)  # код 0 (<R>) разбирается в самом цикле шага
        rows.append((code, name(code), target, h1))
    # тела функций ARM9: размер и вызываемые (bl) функции
    import glob
    funcs = {}
    for f in sorted(glob.glob(sys.argv[2].rsplit("/", 1)[0] + "/p_arm9_[0-9][0-9][0-9].c")):
        cur = None
        for l in open(f, encoding="utf-8", errors="replace"):
            m = re.match(r"void p_arm9_[tf]_([0-9A-F]{8})\(Cpu\* c\) \{", l)
            if m:
                cur = int(m.group(1), 16)
                funcs[cur] = [0, []]
            elif cur is not None:
                if l.startswith("}"):
                    cur = None
                    continue
                funcs[cur][0] += 1
                for b in re.findall(r"//\s*[0-9A-F]{8} bl[x]? 0x([0-9A-F]{8})", l):
                    if int(b, 16) not in funcs[cur][1]:
                        funcs[cur][1].append(int(b, 16))
    out = open(sys.argv[3], "w") if len(sys.argv) > 3 else sys.stdout
    out.write("code,name,case_label,handler,lines,callees\n")
    for code, n, t, h in rows:
        size, calls = funcs.get(h & ~1, [0, []]) if h else (0, [])
        out.write(f"{code},{n},{t:08X},{h:08X},{size},{' '.join(f'{c:08X}' for c in calls)}\n" if h else f"{code},{n},{t:08X},,,\n")


main()
