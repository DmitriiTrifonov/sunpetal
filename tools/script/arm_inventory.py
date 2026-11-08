#!/usr/bin/env python3
"""K-188 (Э1.1): что исполняет ARM — функции, замеченные `calls` в сценариях, с размером (строк C), обращениями к железу DS
(по литералам адресов в теле функции) и подсистемой обработчика команды (если функция — обработчик).

  arm_inventory.py CALLS_DIR OUT_CSV OUT_MD
CALLS_DIR: файлы calls_<сцена>.tsv (вывод `calls FRAMES FILE` из sunpetal-debug: cpu, ключ, входов, vblank, lr, аргументы).
Нужен generated/recomp (не в git). Воспроизведение: tools/script/arm_inventory.sh.
"""
import csv, glob, os, re, sys, collections

REPO = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
GEN = os.path.join(REPO, "generated", "recomp")

# диапазоны регистров и памяти DS -> класс привязки к железу (литерал адреса в теле функции)
HW = [
    (0x04000000, 0x04000070, "2D-A"), (0x04000070, 0x040000B0, "2D-A"), (0x040000B0, 0x04000100, "DMA"),
    (0x04000100, 0x04000110, "таймеры"), (0x04000130, 0x04000134, "ввод"), (0x04000180, 0x04000190, "IPC"),
    (0x040001A0, 0x040001C0, "картридж"), (0x04000200, 0x04000240, "IRQ"), (0x04000240, 0x04000250, "VRAM-банки"),
    (0x04000280, 0x040002C0, "DIV/SQRT"), (0x04000300, 0x04000320, "система"), (0x04000320, 0x04000700, "3D (GX)"),
    (0x04001000, 0x04001070, "2D-B"), (0x04000400, 0x04000500, "3D (GX)"), (0x04000500, 0x04000600, "звук"),
    (0x05000000, 0x06000000, "палитры"), (0x06000000, 0x07000000, "VRAM"), (0x07000000, 0x08000000, "OAM"),
]
LIT = re.compile(r"0x(0[4-7][0-9A-Fa-f]{6})u")


def hwclass(a):
    for lo, hi, n in HW:
        if lo <= a < hi:
            return n
    return None


def load_bodies():
    """{(module, key): (строк, {классы железа})} по generated/recomp/p_arm9*_NNN.c и p_arm7_NNN.c."""
    out = {}
    for path in sorted(glob.glob(os.path.join(GEN, "p_arm[79]*_[0-9][0-9][0-9].c"))):
        mod = re.sub(r"_[0-9]+\.c$", "", os.path.basename(path))[2:]  # arm9, arm9_ov4, arm7
        text = open(path, encoding="utf-8", errors="replace").read()
        for m in re.finditer(r"^void p_\w+?_([ft])_([0-9A-F]{8})\(Cpu\* c\) \{\n(.*?)^\}\n", text, re.S | re.M):
            key = int(m.group(2), 16) | (1 if m.group(1) == "t" else 0)
            body = m.group(3)
            cls = {hwclass(int(l, 16)) for l in LIT.findall(body)} - {None}
            out.setdefault((mod, key), (body.count("\n") + 1, cls))
    return out


def load_handlers():
    """{ключ обработчика: (тег, подсистема)} по docs/native-script-dispatch.csv и таблице §8 NATIVE-SCRIPT.md."""
    sub = {}
    for l in open(os.path.join(REPO, "docs", "NATIVE-SCRIPT.md"), encoding="utf-8"):
        m = re.match(r"\| ([^|]+) \| (\d+) \| (\d+) \| (<.*>) \|\s*$", l)
        if m:
            for tag in re.findall(r"<\w+>", m.group(4)):
                sub[tag] = m.group(1).strip()
    h = {}
    for r in csv.DictReader(open(os.path.join(REPO, "docs", "native-script-dispatch.csv"), encoding="utf-8")):
        if r["handler"]:
            h[int(r["handler"], 16) | 1] = (r["name"], sub.get(r["name"], "служебный"))
    return h


def main(calls_dir, out_csv, out_md):
    bodies = load_bodies()
    handlers = load_handlers()
    scenes = {}
    for p in sorted(glob.glob(os.path.join(calls_dir, "calls_*.tsv"))):
        scenes[os.path.basename(p)[6:-4]] = [l.rstrip("\n").split("\t") for l in open(p)]
    names = list(scenes)
    # ARM9: в основном модуле; иначе — оверлей (ключи между оверлеями совпадают, берём ближайший найденный)
    ovl = [m for m in {k[0] for k in bodies} if m.startswith("arm9_ov")]
    rows = collections.OrderedDict()
    for sc, lst in scenes.items():
        for r in lst:
            cpu, key, n = int(r[0]), int(r[1], 16), int(r[2])
            e = rows.setdefault((cpu, key), {"calls": collections.Counter()})
            e["calls"][sc] += n
    out = []
    for (cpu, key), e in sorted(rows.items()):
        if cpu == 7:
            mod, b = "arm7", bodies.get(("arm7", key))
        else:
            mod, b = "arm9", bodies.get(("arm9", key))
            if b is None:
                for o in sorted(ovl):
                    if (o, key) in bodies:
                        mod, b = o, bodies[(o, key)]
                        break
        lines, cls = b if b else (0, set())
        tag, sub = handlers.get(key, ("", "")) if cpu == 9 else ("", "")
        out.append({"cpu": cpu, "key": f"{key:08X}", "module": mod if b else ("?" if cpu == 9 else "arm7?"), "lines": lines,
                    "hw": "|".join(sorted(cls)), "handler": tag, "subsystem": sub,
                    **{f"calls_{s}": e["calls"].get(s, 0) for s in names}})
    with open(out_csv, "w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=list(out[0]))
        w.writeheader()
        w.writerows(out)
    md = []
    md.append("# Инвентаризация ARM (K-188, Э1.1)\n")
    md.append("Сценарии: " + ", ".join(names) + ". Функции — входы, замеченные `calls` (ARM9 и ARM7); строки C и классы железа — из `generated/recomp` (статически, по литералам адресов 0x04…0x07 в теле). Таблица по функциям — `docs/native-engine-inventory.csv`; воспроизведение — `tools/script/arm_inventory.sh`.\n")
    def agg(sel):
        fs = [r for r in out if sel(r)]
        return len(fs), sum(r["lines"] for r in fs)
    md.append("## Сводка\n")
    md.append("| Группа | функций | строк C |\n|---|--:|--:|")
    for label, sel in [("ARM9 всего (замечено)", lambda r: r["cpu"] == 9), ("ARM9 основной модуль", lambda r: r["module"] == "arm9"),
                       ("ARM9 оверлеи", lambda r: r["module"].startswith("arm9_ov")), ("ARM9 не найдено в генерации", lambda r: r["module"] == "?"),
                       ("ARM9 обработчики команд сценария", lambda r: r["handler"]), ("ARM9 с обращением к железу DS", lambda r: r["cpu"] == 9 and r["hw"]),
                       ("ARM7 всего (звук, касание, RTC)", lambda r: r["cpu"] == 7)]:
        n, l = agg(sel)
        md.append(f"| {label} | {n} | {l} |")
    md.append("\n## По сценариям (ARM9)\n")
    md.append("| Сцена | функций | строк C | с обращением к железу | входов всего |\n|---|--:|--:|--:|--:|")
    for s in names:
        fs = [r for r in out if r["cpu"] == 9 and r[f"calls_{s}"]]
        md.append(f"| {s} | {len(fs)} | {sum(r['lines'] for r in fs)} | {sum(1 for r in fs if r['hw'])} | {sum(r[f'calls_{s}'] for r in fs)} |")
    only = collections.Counter()
    for r in out:
        if r["cpu"] == 9:
            seen = [s for s in names if r[f"calls_{s}"]]
            if len(seen) == 1:
                only[seen[0]] += 1
    md.append("\nФункции, замеченные только в одном сценарии (ARM9): " + ", ".join(f"{s} {only[s]}" for s in names) + ". Общих для всех: "
              + str(sum(1 for r in out if r["cpu"] == 9 and all(r[f"calls_{s}"] for s in names))) + ".\n")
    md.append("## Классы железа DS в замеченных функциях ARM9\n")
    c = collections.Counter()
    cl = collections.Counter()
    for r in out:
        if r["cpu"] == 9:
            for h in filter(None, r["hw"].split("|")):
                c[h] += 1
                cl[h] += r["lines"]
    md.append("| Класс | функций | строк C |\n|---|--:|--:|")
    for h, n in c.most_common():
        md.append(f"| {h} | {n} | {cl[h]} |")
    md.append("\n## Обработчики команд по подсистемам (замеченные)\n")
    sc = collections.Counter(); sl = collections.Counter()
    for r in out:
        if r["handler"]:
            sc[r["subsystem"]] += 1; sl[r["subsystem"]] += r["lines"]
    md.append("| Подсистема | обработчиков в сценариях | строк C |\n|---|--:|--:|")
    for k, n in sc.most_common():
        md.append(f"| {k} | {n} | {sl[k]} |")
    md.append("\n## Самые крупные замеченные функции ARM9 (строк C)\n")
    md.append("| Ключ | модуль | строк | железо | входов за все сцены |\n|---|---|--:|---|--:|")
    for r in sorted((r for r in out if r["cpu"] == 9), key=lambda r: -r["lines"])[:15]:
        md.append(f"| {r['key']} | {r['module']} | {r['lines']} | {r['hw'] or '—'} | {sum(r[f'calls_{s}'] for s in names)} |")
    open(out_md, "w", encoding="utf-8").write("\n".join(md) + "\n")
    print("\n".join(md))


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    main(*sys.argv[1:])
