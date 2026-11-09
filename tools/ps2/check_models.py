#!/usr/bin/env python3
"""Автопроверки моделей PS2 (K-183): метрики формы по OBJ каталога K-125, без глаз, без PIL и numpy.

    check_models.py CATALOGDIR [--csv OUT.csv] [--json OUT.json] [--report OUT.html] [--check]

Читает `CATALOGDIR/<АРХИВ>/NNN.obj` и `CATALOGDIR/index.csv`; считает метрики каждой модели (см. `metrics`),
дописывает их колонками к строкам индекса (`--csv`) и печатает таблицу со статусом. Статус `ok` / `bad`
выводит `verdict` по порогам; эталонный список (`REFERENCE`) показывает, где пороги ошибаются. Порог подбирался
по нескольким моделям: ручная проверка автора остаётся финальной (docs/PS2-MODEL-CHECKS.md).
"""
import csv
import json
import math
import os
import sys

# Квант координат для склейки вершин (доля диагонали bbox): в OBJ у каждого треугольника свои три вершины
QUANT = 1e-4
# Допуск зеркального совпадения (доля диагонали)
MIRROR_TOL = 0.02
# Предел вершин для симметрии (равномерная выборка): хватает для оценки, держит время на разумном уровне
SAMPLE = 1500
BIG = 1e5


def read_obj(path):
    """(вершины по треугольникам, uv) — по три вершины на треугольник, как их записывает каталог."""
    v, vt, tris, uvs = [], [], [], []
    with open(path) as f:
        for ln in f:
            if ln.startswith("v "):
                v.append(tuple(float(x) for x in ln.split()[1:4]))
            elif ln.startswith("vt "):
                vt.append(tuple(float(x) for x in ln.split()[1:3]))
            elif ln.startswith("f "):
                idx = [p.split("/") for p in ln.split()[1:4]]
                tris.append(tuple(int(p[0]) - 1 for p in idx))
                if len(idx[0]) > 1 and idx[0][1]:
                    uvs.append(tuple(int(p[1]) - 1 for p in idx))
    return v, vt, tris, uvs


def bbox(pts):
    return [(min(p[a] for p in pts), max(p[a] for p in pts)) for a in range(3)]


def dist(a, b):
    return math.sqrt(sum((x - y) ** 2 for x, y in zip(a, b)))


def components(v, tris, diag):
    """Связные компоненты по общим (с точностью QUANT·diag) вершинам: список (индексы треугольников)."""
    q = max(diag * QUANT, 1e-9)
    key = {}
    parent = []

    def find(x):
        while parent[x] != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x

    ids = []
    for p in v:
        k = (round(p[0] / q), round(p[1] / q), round(p[2] / q))
        if k not in key:
            key[k] = len(parent)
            parent.append(len(parent))
        ids.append(key[k])
    for t in tris:
        a, b, c = (find(ids[i]) for i in t)
        parent[b] = a
        parent[find(c)] = find(a)
    comps = {}
    for n, t in enumerate(tris):
        comps.setdefault(find(ids[t[0]]), []).append(n)
    return sorted(comps.values(), key=len, reverse=True)


def comp_boxes(v, tris, comps):
    out = []
    for c in comps:
        pts = [v[i] for n in c for i in tris[n]]
        out.append((len(c), bbox(pts)))
    return out


def box_iou(a, b):
    inter = 1.0
    union = 1.0
    for (a0, a1), (b0, b1) in zip(a, b):
        lo, hi = max(a0, b0), min(a1, b1)
        inter *= max(hi - lo, 0.0)
        union *= max(max(a1, b1) - min(a0, b0), 1e-12)
    va = 1.0
    vb = 1.0
    for (a0, a1), (b0, b1) in zip(a, b):
        va *= max(a1 - a0, 1e-12)
        vb *= max(b1 - b0, 1e-12)
    return inter / max(va + vb - inter, 1e-18)


def stacked_parts(boxes, diag):
    """Сколько частей (кроме главной) имеют «двойника»: часть с почти тем же bbox (IoU ≥ 0.6) и сопоставимым числом
    треугольников. Целая модель их почти не имеет; двери и колёса, сложенные в центр, дают пары."""
    big = [b for b in boxes[1:] if b[0] >= 4 and max(r[1] - r[0] for r in b[1]) >= 0.05 * diag]
    n = 0
    for i, a in enumerate(big):
        for j, b in enumerate(big):
            if i != j and 0.5 <= a[0] / b[0] <= 2 and box_iou(a[1], b[1]) >= 0.6:
                n += 1
                break
    return n, len(big)


def metrics(path):
    v, vt, tris, uvs = read_obj(path)
    m = {"v": len(v), "tris": len(tris)}
    if not tris:
        m.update(integrity=0.0, nan=0, uv_bad=0, degenerate=0.0, long_edge=0.0, comps=0, stacked=0, parts=0, main_share=0.0,
                 origin_mass=0.0, sym_x=0.0, sym_y=0.0, sym_z=0.0, sym_best=0.0, diag=0.0, dims="")
        return m
    pts = [p for p in v]
    nan = sum(1 for p in pts if not all(math.isfinite(c) for c in p))
    pts = [p for p in pts if all(math.isfinite(c) for c in p)]
    bb = bbox(pts)
    diag = max(dist((bb[0][0], bb[1][0], bb[2][0]), (bb[0][1], bb[1][1], bb[2][1])), 1e-9)
    oob = sum(1 for t in tris for i in t if not 0 <= i < len(v))
    uv_bad = sum(1 for p in vt if not all(math.isfinite(c) and -64 <= c <= 64 for c in p))
    m.update(nan=nan, uv_bad=uv_bad, diag=round(diag, 3), dims=" ".join("%.4g" % (b[1] - b[0]) for b in bb))
    m["integrity"] = 1.0 if (nan == 0 and oob == 0 and uv_bad == 0 and diag < BIG * 4) else 0.0
    if m["integrity"] < 1.0:
        # битая запись: дальше считать нечего (NaN и индексы вне границ ломают склейку), статус `bad` по integrity
        m.update(degenerate=0.0, long_edge=0.0, comps=0, stacked=0, parts=0, main_share=0.0, origin_mass=0.0,
                 sym_x=0.0, sym_y=0.0, sym_z=0.0, sym_best=0.0)
        return m
    # вырожденные и длинные рёбра
    longest, degen = [], 0
    for t in tris:
        a, b, c = (v[i] for i in t)
        e = (dist(a, b), dist(b, c), dist(c, a))
        longest.append(max(e))
        s = sum(e) / 2
        area2 = max(s * (s - e[0]) * (s - e[1]) * (s - e[2]), 0.0)
        if area2 <= (diag * 1e-6) ** 4:
            degen += 1
    med = sorted(longest)[len(longest) // 2]
    m["degenerate"] = round(degen / len(tris), 4)
    # доля треугольников с самым длинным ребром > 6 медиан (неверная сборка полос: осколки)
    m["long_edge"] = round(sum(1 for x in longest if x > 6 * med) / len(tris), 4) if med > 0 else 0.0
    comps = components(v, tris, diag)
    m["comps"] = len(comps)
    boxes = comp_boxes(v, tris, comps)
    m["stacked"], m["parts"] = stacked_parts(boxes, diag)
    m["main_share"] = round(len(comps[0]) / len(tris), 4)
    # масса вершин около начала координат модели (части в локальных координатах лежат кучей в нуле)
    inside = all(b[0] <= 0 <= b[1] for b in bb)
    if inside:
        r = 0.06 * diag
        m["origin_mass"] = round(sum(1 for p in pts if dist(p, (0, 0, 0)) <= r) / len(pts), 4)
    else:
        m["origin_mass"] = 0.0
    # симметрия: доля зеркальных пар по каждой оси относительно центра bbox
    uniq = list({(round(p[0], 3), round(p[1], 3), round(p[2], 3)) for p in pts})
    if len(uniq) > SAMPLE:
        step = len(uniq) / SAMPLE
        uniq = [uniq[int(i * step)] for i in range(SAMPLE)]
    full = {(round(p[0], 3), round(p[1], 3), round(p[2], 3)) for p in pts}
    tol = MIRROR_TOL * diag
    sym = []
    for a in range(3):
        c = (bb[a][0] + bb[a][1]) / 2
        # образ ищем среди всех точек, не только среди выборки
        sym.append(round(mirror_score_full(uniq, full, a, c, tol), 4))
    m["sym_x"], m["sym_y"], m["sym_z"] = sym
    m["sym_best"] = max(sym)
    return m


def mirror_score_full(sample, full, axis, center, tol):
    cell = max(tol, 1e-9)
    grid = {}
    for p in full:
        grid.setdefault((int(p[0] // cell), int(p[1] // cell), int(p[2] // cell)), []).append(p)
    hit = 0
    for p in sample:
        m = list(p)
        m[axis] = 2 * center - m[axis]
        k = (int(m[0] // cell), int(m[1] // cell), int(m[2] // cell))
        found = False
        for dx in (-1, 0, 1):
            for dy in (-1, 0, 1):
                for dz in (-1, 0, 1):
                    for q in grid.get((k[0] + dx, k[1] + dy, k[2] + dz), ()):
                        if dist(m, q) <= tol:
                            found = True
                            break
                    if found:
                        break
                if found:
                    break
            if found:
                break
        hit += found
    return hit / len(sample)


# Эталонный список (K-183): (архив, запись) → ожидаемый статус. Заведомо верные: купола неба, планы помещений (автор,
# листы K-125); заведомо сломанные: машины, мотоцикл и персонажи OP#13…22 (части не на местах), россыпь осколков OP#21, #43.
# Новая поломка в «верной» модели даёт красный `--check`; известные поломки показывают прогресс, а не роняют тест.
REFERENCE_OK = [("OP", n) for n in (1, 2, 3, 4, 7, 8, 9)] + [("TORUKURO", 66)] + [
    ("R00", n) for n in (1, 2, 3, 4, 5, 6, 8, 10, 12, 14, 15, 16, 17, 18, 22, 23, 24, 25, 30, 31, 45)]
REFERENCE_BAD = [("OP", n) for n in (13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 43)]

# Пороги подобраны по прогону 2026-11-09 (125 моделей): статичные модели дают stacked 0–2, машины и персонажи 4–16
STACKED_BAD = 4
STACKED_CHECK = 2
SHARD_TRIS_PER_COMP = 3.0
LONG_EDGE_CHECK = 0.35  # планы R00#14 (0.30) и #31 (0.20) с длинными стенами верны


def verdict(m):
    """(статус, причина): `ok`, `check` (смотреть глазами) или `bad`."""
    if m.get("integrity", 1.0) < 1.0:
        return "bad", "integrity: битые индексы, NaN, UV или габариты"
    if m["tris"] == 0:
        return "bad", "empty"
    if m["comps"] >= 3 and m["tris"] / m["comps"] < SHARD_TRIS_PER_COMP:
        return "bad", "shards: треугольники россыпью (%.1f на компоненту)" % (m["tris"] / m["comps"])
    if m["stacked"] >= STACKED_BAD:
        return "bad", "stacked: %d частей-двойников из %d (части не на местах, локальные координаты?)" % (m["stacked"], m["parts"])
    if m["stacked"] >= STACKED_CHECK:
        return "check", "stacked: %d частей-двойников из %d" % (m["stacked"], m["parts"])
    if m["long_edge"] >= LONG_EDGE_CHECK and m["tris"] >= 100:
        return "check", "long_edge: %.0f%% треугольников с длинным ребром" % (100 * m["long_edge"])
    return "ok", ""


def report_html(out, cat, path):
    """report.html: миниатюры каталога с подписью (архив, запись) и статусом; красные и спорные — наверху."""
    color = {"bad": "#c0392b", "check": "#d4a017", "ok": "#2e8b57"}
    order = {"bad": 0, "check": 1, "ok": 2}
    cells = []
    for m in sorted(out, key=lambda m: (order[m["status"]], m["arch"], m["entry"])):
        img = os.path.relpath(os.path.join(cat, m["arch"], "%03d_view.png" % m["entry"]), os.path.dirname(path))
        cells.append(
            '<div style="border:4px solid %s;display:inline-block;margin:4px;padding:2px;width:490px;font:12px monospace">'
            '<b>%s#%d</b> %s<br>%s<br><img src="%s" width="480"></div>' % (
                color[m["status"]], m["arch"], m["entry"], m["status"].upper(), m["reason"] or "", img))
    n = {k: sum(1 for m in out if m["status"] == k) for k in color}
    open(path, "w", encoding="utf-8").write(
        "<!doctype html><meta charset=utf-8><title>K-183: проверки моделей PS2</title>"
        "<h3>K-183: %d моделей — красных %d, спорных %d, зелёных %d</h3>%s" % (len(out), n["bad"], n["check"], n["ok"], "".join(cells)))


def check_reference(out):
    """Строки расхождений с эталонным списком (пусто — всё как ожидалось)."""
    by = {(m["arch"], m["entry"]): m for m in out}
    bad = []
    for k in REFERENCE_OK:
        if k in by and by[k]["status"] != "ok":
            bad.append("%s#%d должна быть зелёной, а %s: %s" % (k[0], k[1], by[k]["status"], by[k]["reason"]))
    for k in REFERENCE_BAD:
        if k in by and by[k]["status"] != "bad":
            bad.append("%s#%d должна быть красной, а %s" % (k[0], k[1], by[k]["status"]))
    return bad


def main():
    args = sys.argv[1:]
    if not args or args[0].startswith("-"):
        print(__doc__)
        return 2
    cat = args[0]
    opt = {a: args[i + 1] for i, a in enumerate(args) if a in ("--csv", "--json", "--report") and i + 1 < len(args)}
    rows = list(csv.DictReader(open(os.path.join(cat, "index.csv"), encoding="utf-8")))
    out = []
    for r in rows:
        arch, n = r["архив"], int(r["запись"])
        path = os.path.join(cat, arch, "%03d.obj" % n)
        if not os.path.exists(path):
            continue
        m = metrics(path)
        m.update(arch=arch, entry=n)
        m["status"], m["reason"] = verdict(m)
        out.append(m)
        print("%-5s %-9s %3d v=%5d tris=%5d comps=%4d stacked=%3d/%-3d main=%.2f long=%.2f origin=%.2f sym=%.2f/%.2f/%.2f" % (
            m["status"], arch, n, m["v"], m["tris"], m["comps"], m["stacked"], m["parts"], m["main_share"], m["long_edge"], m["origin_mass"],
            m["sym_x"], m["sym_y"], m["sym_z"]), flush=True)
    if "--report" in opt:
        report_html(out, cat, opt["--report"])
    n = {k: sum(1 for m in out if m["status"] == k) for k in ("bad", "check", "ok")}
    print("моделей %d: красных %d, спорных %d, зелёных %d" % (len(out), n["bad"], n["check"], n["ok"]))
    rc = 0
    if "--check" in args:
        miss = check_reference(out)
        for ln in miss:
            print("ЭТАЛОН:", ln)
        print("эталон: %s" % ("расхождений нет" if not miss else "расхождений %d" % len(miss)))
        rc = 1 if miss else 0
    if "--json" in opt:
        json.dump(out, open(opt["--json"], "w"), ensure_ascii=False, indent=1)
    if "--csv" in opt:
        cols = ["v", "tris", "comps", "stacked", "parts", "main_share", "long_edge", "degenerate", "origin_mass", "sym_x", "sym_y", "sym_z",
                "sym_best", "integrity", "status", "reason"]
        by = {(m["arch"], m["entry"]): m for m in out}
        with open(opt["--csv"], "w", newline="", encoding="utf-8") as f:
            w = csv.writer(f)
            w.writerow(list(rows[0].keys()) + ["m_" + c for c in cols])
            for r in rows:
                m = by.get((r["архив"], int(r["запись"])))
                w.writerow(list(r.values()) + ([m.get(c, "") for c in cols] if m else [""] * len(cols)))
    return rc


if __name__ == "__main__":
    sys.exit(main())
