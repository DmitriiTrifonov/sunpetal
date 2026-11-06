#!/usr/bin/env python3
"""K-160: опись тегов-команд сценариев сцен (`*.txt.z`: LZ10, UTF-16, теги `<ИМЯ>…</ИМЯ>`).

    inventory.py DATA_DIR OUT.csv      DATA_DIR = папка `data` распакованного ROM (только чтение)

Пишет CSV: тег, число вхождений, число файлов, распределение числа строк-аргументов. Текст игры в вывод не идёт.
"""
import collections
import csv
import glob
import re
import sys


def lz10(b):
    assert b[0] == 0x10, "не LZ10"
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


def main(root, out):
    cnt, files, args = collections.Counter(), collections.defaultdict(set), collections.defaultdict(collections.Counter)
    n = bad = 0
    for f in sorted(glob.glob(root + "/**/*.txt.z", recursive=True)):
        try:
            t = lz10(open(f, "rb").read()).decode("utf-16")
        except Exception:
            bad += 1
            continue
        n += 1
        for m in re.finditer(r"<([A-Z0-9_]+)>\r?\n(.*?)</\1>", t, re.S):
            name = m.group(1)
            cnt[name] += 1
            files[name].add(f)
            lines = [x for x in re.sub(r"//.*", "", m.group(2)).split("\n") if x.strip()]
            args[name][len(lines)] += 1
    with open(out, "w", newline="") as fh:
        w = csv.writer(fh)
        w.writerow(["tag", "count", "files", "arg_lines"])
        for k, v in cnt.most_common():
            w.writerow([k, v, len(files[k]), " ".join(f"{a}:{c}" for a, c in sorted(args[k].items()))])
    print(f"{n} файлов, не разобрано {bad}, тегов {len(cnt)}, вхождений {sum(cnt.values())}")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
