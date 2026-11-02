#!/usr/bin/env python3
"""Сбор материала PS2-разведки из PCSX2 в `artifacts/ps2-recon/` (не в git).

    ingest.py scan                        забрать новые снимки (snaps), кадры из сохранений (sstates)
                                          и записи PCSX2 (videos) в inbox/. Записи экрана с Рабочего стола не
                                          читаем: macOS выгружает их в iCloud, чтение зависает. Пишите видео
                                          встроенной записью PCSX2 (в videos/).
    ingest.py tag FILE.. --as РЕЖИМ       переложить файлы из inbox/ в каталог режима, обновить манифест
    ingest.py frames VIDEO [--scene T|--fps N]   кадры из записи (по смене сцены T=0.3 или N кадров/с)
    ingest.py sheet DIR [--cols 4]        контактный лист каталога (для просмотра за один раз)
    ingest.py list                        манифест кратко

Режимы см. tools/ps2/modes.md. Дубликаты отсекаются по SHA-1. Исходники PCSX2 не трогаем: только копируем.
"""
import glob
import hashlib
import json
import os
import shutil
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import p2s  # noqa: E402

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
OUT = os.path.join(ROOT, "artifacts", "ps2-recon")
MAN = os.path.join(OUT, "manifest.json")
PCSX2 = os.path.expanduser("~/Library/Application Support/PCSX2")
SOURCES = [
    (os.path.join(PCSX2, "snaps", "*.png"), "snap"),
    (os.path.join(PCSX2, "videos", "*.mp4"), "video"),
]


def load():
    return json.load(open(MAN)) if os.path.exists(MAN) else {}


def save(m):
    os.makedirs(OUT, exist_ok=True)
    json.dump(m, open(MAN, "w"), ensure_ascii=False, indent=1, sort_keys=True)


def add(m, data, name, kind, src, mtime):
    h = hashlib.sha1(data).hexdigest()
    if h in m:
        return False
    d = os.path.join(OUT, "inbox")
    os.makedirs(d, exist_ok=True)
    open(os.path.join(d, name), "wb").write(data)
    m[h] = {"file": f"inbox/{name}", "kind": kind, "src": src, "mtime": int(mtime), "mode": "inbox"}
    return True


def scan(args):
    m = load()
    new = 0
    for pat, kind in SOURCES:
        for f in sorted(glob.glob(pat)):
            name = os.path.basename(f).replace("Flower, Sun, and Rain_SLPS-25034_", "snap-")
            name = name.replace(" ", "_").replace("—", "-")
            try:
                new += add(m, open(f, "rb").read(), name, kind, f, os.path.getmtime(f))
            except OSError as e:  # например, файл Рабочего стола выгружен в iCloud или нет доступа
                print(f"пропуск {f}: {e}", file=sys.stderr)
    for f in sorted(glob.glob(os.path.join(PCSX2, "sstates", "*.p2s"))):
        slot = f.rsplit(".", 2)[-2]
        try:
            shot = p2s.read(p2s.entries(f), "Screenshot.png")
        except KeyError:
            continue
        stamp = int(os.path.getmtime(f))
        new += add(m, shot, f"state-slot{slot}-{stamp}.png", "state", f, stamp)
    save(m)
    print(f"новых: {new}, всего в манифесте: {len(m)}")


def tag(args):
    i = args.index("--as")
    mode, files = args[i + 1], args[:i]
    m = load()
    os.makedirs(os.path.join(OUT, mode), exist_ok=True)
    for f in files:
        src = f if os.path.isabs(f) else os.path.join(OUT, "inbox", f)
        rel = os.path.relpath(src, OUT)
        shutil.move(src, os.path.join(OUT, mode, os.path.basename(src)))
        for v in m.values():
            if v["file"] == rel:
                v["file"], v["mode"] = f"{mode}/{os.path.basename(src)}", mode
    save(m)


def frames(args):
    v = args[0]
    d = os.path.join(OUT, "frames", os.path.splitext(os.path.basename(v))[0])
    os.makedirs(d, exist_ok=True)
    if "--fps" in args:
        vf = f"fps={args[args.index('--fps') + 1]}"
    else:
        vf = f"select='gt(scene,{args[args.index('--scene') + 1] if '--scene' in args else '0.3'})'"
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", v, "-vf", vf, "-fps_mode", "vfr",
                    os.path.join(d, "f%04d.png")], check=True)
    print(d, len(os.listdir(d)), "кадров")


def sheet(args):
    d = args[0]
    cols = int(args[args.index("--cols") + 1]) if "--cols" in args else 4
    n = len([f for f in glob.glob(os.path.join(d, "*.png")) if not f.endswith("_sheet.png")])
    out = os.path.join(d, "_sheet.png")
    if os.path.exists(out):
        os.remove(out)
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-pattern_type", "glob", "-i", os.path.join(d, "*.png"),
                    "-vf", f"scale=320:-1,tile={cols}x{max(1, -(-n // cols))}", "-frames:v", "1", out], check=True)
    print(out)


def main():
    a = sys.argv[1:]
    cmds = {"scan": scan, "tag": tag, "frames": frames, "sheet": sheet}
    if not a or a[0] not in cmds and a[0] != "list":
        raise SystemExit(__doc__)
    if a[0] == "list":
        for v in sorted(load().values(), key=lambda v: v["mtime"]):
            print(v["mode"], v["kind"], v["file"])
    else:
        cmds[a[0]](a[1:])


if __name__ == "__main__":
    main()
