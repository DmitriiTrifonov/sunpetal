#!/usr/bin/env python3
"""Канбан Sunpetal: tasks.json -> docs/KANBAN.md. RICE = R*I*C/E, у тем менеджера ×1.5 (boost).

  kanban.py render
  kanban.py add "название" --r 6 --i 1 --c 0.8 --e 3 [--theme] [--p0] [--col backlog] [--note ...]
  (add ... --after K-001,K-002: не выше этих задач)
  kanban.py move ID колонка          (backlog|doing|check|blocked|done)
  kanban.py p0 ID                    (поднять в критичные)
"""
import argparse, json, pathlib, datetime

ROOT = pathlib.Path(__file__).resolve().parents[2]
DB = pathlib.Path(__file__).with_name("tasks.json")
OUT = ROOT / "docs" / "KANBAN.md"
BOOST = 1.5
WIP = 2
COLS = ["doing", "check", "backlog", "blocked", "done"]
TITLE = {"doing": "В работе", "check": "Проверка (ручная, нужен автор)", "backlog": "Очередь (по RICE)",
         "blocked": "Ждёт слова автора / отложено", "done": "Сделано"}


def load():
    return json.loads(DB.read_text())


def save(d):
    DB.write_text(json.dumps(d, ensure_ascii=False, indent=1))


def score(t):
    s = t["r"] * t["i"] * t["c"] / t["e"]
    return s * (BOOST if t.get("theme") else 1)


def eff(t, by, seen=()):
    """RICE с учётом «после»: не выше предшественницы (если та ещё не сделана)."""
    v = score(t)
    for a in t.get("after", []):
        u = by.get(a)
        if u and u["col"] != "done" and a not in seen:
            v = min(v, eff(u, by, seen + (t["id"],)) - 0.001)
    return v


def render():
    d = load()
    ts = d["tasks"]
    by = {t["id"]: t for t in ts}
    L = ["# Канбан Sunpetal", "",
         f"Обновлён {datetime.date.today()} (`python3 tools/kanban/kanban.py render`; данные — `tools/kanban/tasks.json`).", "",
         "## Правила", "",
         "- **Порядок.** Берём сверху «Очереди»: сначала критичные (P0), затем по убыванию RICE. Не в порядке — только по слову автора.",
         "- **RICE = R·I·C / E.** R — охват 1–10 (доля игроков и сессий), I — влияние 0,25 / 0,5 / 1 / 2 / 3, C — уверенность 0,5–1, E — трудоёмкость из `roadmap.json` (1, 2, 3, 5, 8, 13).",
         f"- **Приоритет менеджера:** PS2-ассеты, графика и шрифты (метка ★) получают ×{BOOST}.",
         "- **Критичное (P0):** падение, потеря сохранения, нельзя пройти игру, поломка сборки или CI. Встаёт выше всех, идёт перед очередью, остальное ждёт.",
         f"- **WIP ≤ {WIP}** в «В работе». «Проверка» не считается: ждёт окна/геймпада автора.",
         "- **Новая задача:** спросить приоритет у автора; если не ответил — выставляю R/I/C/E сам и пишу, куда она встала. Критичная — сразу P0.",
         "- Закрытие — скилл `close-step`, замер — `tools/roadmap/timelog.py`.", ""]
    for col in COLS:
        items = [t for t in ts if t["col"] == col]
        if col == "done":
            items = items[-8:]
        if col in ("backlog", "blocked"):
            items.sort(key=lambda t: (not t.get("p0"), -eff(t, by)))
        L += [f"## {TITLE[col]} ({len(items)})", ""]
        if not items:
            L += ["—", ""]
            continue
        if col in ("backlog", "blocked"):
            L += ["| # | ID | Задача | P0 | ★ | R | I | C | E | RICE | Примечание |", "|--:|---|---|:-:|:-:|--:|--:|--:|--:|--:|---|"]
            for n, t in enumerate(items, 1):
                L.append(f"| {n} | {t['id']} | {t['t']} | {'**P0**' if t.get('p0') else ''} | {'★' if t.get('theme') else ''} | {t['r']} | {t['i']} | {t['c']} | {t['e']} | {score(t):.2f}{' (после '+','.join(t['after'])+')' if t.get('after') else ''} | {t.get('note','')} |")
        else:
            for t in items:
                L.append(f"- **{t['id']}** {t['t']}" + (f" — {t['note']}" if t.get("note") else ""))
        L.append("")
    OUT.write_text("\n".join(L))
    print(f"{OUT}: {len(ts)} задач")


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("render")
    a = sub.add_parser("add")
    a.add_argument("title"); a.add_argument("--r", type=float, default=5); a.add_argument("--i", type=float, default=1)
    a.add_argument("--c", type=float, default=0.8); a.add_argument("--e", type=float, default=3)
    a.add_argument("--theme", action="store_true"); a.add_argument("--p0", action="store_true")
    a.add_argument("--after", default=""); a.add_argument("--col", default="backlog"); a.add_argument("--note", default="")
    m = sub.add_parser("move"); m.add_argument("id"); m.add_argument("col", choices=COLS)
    p = sub.add_parser("p0"); p.add_argument("id")
    x = ap.parse_args()
    d = load()
    if x.cmd == "add":
        d["seq"] += 1
        t = {"id": f"K-{d['seq']:03}", "t": x.title, "col": x.col, "r": x.r, "i": x.i, "c": x.c, "e": x.e, "note": x.note}
        if x.theme: t["theme"] = True
        if x.p0: t["p0"] = True
        if x.after: t["after"] = x.after.split(",")
        d["tasks"].append(t); save(d); print(t["id"], f"RICE {score(t):.2f}")
    elif x.cmd in ("move", "p0"):
        t = next(t for t in d["tasks"] if t["id"] == x.id)
        if x.cmd == "move":
            if x.col == "doing" and sum(1 for u in d["tasks"] if u["col"] == "doing") >= WIP:
                raise SystemExit(f"WIP {WIP}: сначала закрыть или убрать задачу из «В работе»")
            t["col"] = x.col
        else:
            t["p0"] = True
        save(d)
    render()


main()
