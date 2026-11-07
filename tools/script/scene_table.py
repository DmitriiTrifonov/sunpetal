#!/usr/bin/env python3
"""K-163: таблица сцен из текста сценариев (`Chapter/Pack/Cnn/*.txt.z`): признаки, которые видны без картинки.

    scene_table.py DATA_DIR OUT.csv      DATA_DIR = папка `data` распакованного ROM (только чтение)

Столбцы: scene, commands, text_ex (число реплик), device (команд CALLKATHARINE; в них устройство Катарины и
головоломки), puzzles (первые аргументы CALLKATHARINE через пробел), screen_fx (SETSCREEN), movie (PLAYMOVIE),
credits (ED_TEXT_START), shooter (SHOOT_START), close_chapter, map (аргумент первого CHANGEMAP/CHANGEMAP2), kind.
`kind` — первая подходящая строка правил: shooter, movie, credits, device (есть CALLKATHARINE), talk (есть реплики),
walk (иначе). Текст реплик в таблицу не идёт.
"""
import csv
import glob
import os
import re
import sys


def lz10(b):
    n = b[1] | b[2] << 8 | b[3] << 16
    o, i = bytearray(), 4
    while len(o) < n:
        f = b[i]
        i += 1
        for k in range(8):
            if len(o) >= n:
                break
            if f & (0x80 >> k):
                x = b[i] << 8 | b[i + 1]
                i += 2
                d = (x & 0xFFF) + 1
                for _ in range((x >> 12) + 3):
                    o.append(o[-d])
            else:
                o.append(b[i])
                i += 1
    return bytes(o[:n])


def args(body):
    return [x.strip() for x in re.sub(r"//[^\n]*", "", body).split("\n") if x.strip()]


def row(name, text):
    cmds = re.findall(r"<([A-Z0-9_]+)>", re.sub(r"//[^\n]*", "", text))
    tags = {}
    for t in cmds:
        tags[t] = tags.get(t, 0) + 1
    puzzles = []
    for m in re.finditer(r"<CALLKATHARINE>(.*?)</CALLKATHARINE>", text, re.S):
        a = args(m.group(1))
        puzzles.append(a[0].split()[0] if a else "")
    target = ""
    for m in re.finditer(r"<CHANGEMAP2?>(.*?)</CHANGEMAP2?>", text, re.S):
        a = args(m.group(1))
        if a:
            target = a[0].split()[0]
            break
    r = {
        "scene": name,
        "commands": len(cmds),
        "text_ex": tags.get("TEXT_EX", 0),
        "device": tags.get("CALLKATHARINE", 0),
        "puzzles": " ".join(puzzles),
        "screen_fx": tags.get("SETSCREEN", 0),
        "movie": tags.get("PLAYMOVIE", 0),
        "credits": tags.get("ED_TEXT_START", 0),
        "shooter": tags.get("SHOOT_START", 0),
        "close_chapter": tags.get("CLOSE_CHAPTER", 0),
        "map": target,
    }
    r["kind"] = "shooter" if r["shooter"] else "movie" if r["movie"] else "credits" if r["credits"] else "device" if r["device"] else "talk" if r["text_ex"] else "walk"
    return r


def main(root, out):
    rows = []
    for f in sorted(glob.glob(root + "/Chapter/Pack/C*/*.txt.z")):
        name = os.path.basename(f)[:-6]
        try:
            text = lz10(open(f, "rb").read()).decode("utf-16")
        except Exception:
            continue
        rows.append(row(name, text))
    cols = ["scene", "commands", "text_ex", "device", "puzzles", "screen_fx", "movie", "credits", "shooter", "close_chapter", "map", "kind"]
    with open(out, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=cols)
        w.writeheader()
        w.writerows(rows)
    kinds = {}
    for r in rows:
        kinds[r["kind"]] = kinds.get(r["kind"], 0) + 1
    print(len(rows), "сцен;", ", ".join(f"{k} {v}" for k, v in sorted(kinds.items())))


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
