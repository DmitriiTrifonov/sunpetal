#!/usr/bin/env python3
"""Замеры времени по этапам и шагам Sunpetal (только stdlib).

  timelog.py start ID "название"   — записать начало работы над этапом/шагом
  timelog.py end ID ["заметка"]    — записать окончание (закрывает последнюю открытую запись ID)
  timelog.py open                  — список открытых записей
  timelog.py history [--cap H] [--lead H]
                                   — пересобрать history.csv из git (источник «git»)
  timelog.py report                — таблица: живые замеры и история

Файлы: timelog.csv (живые, руками не править), history.csv (производный, пересобирается).
Столбцы: id,title,start,end,hours,commits,source,note.
"""
import argparse
import csv
import json
import re
import subprocess
import sys
from datetime import datetime, timedelta
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
LIVE = HERE / "timelog.csv"
HIST = HERE / "history.csv"
RULES = HERE / "history-rules.json"
FIELDS = ["id", "title", "start", "end", "hours", "commits", "source", "note"]


def now():
    return datetime.now().astimezone().isoformat(timespec="seconds")


def read(path):
    if not path.exists():
        return []
    with path.open(encoding="utf-8", newline="") as f:
        return list(csv.DictReader(f))


def write(path, rows):
    with path.open("w", encoding="utf-8", newline="") as f:
        w = csv.DictWriter(f, FIELDS)
        w.writeheader()
        w.writerows({k: r.get(k, "") for k in FIELDS} for r in rows)


def hours_between(a, b):
    return (datetime.fromisoformat(b) - datetime.fromisoformat(a)).total_seconds() / 3600


def cmd_start(a):
    rows = read(LIVE)
    if any(r["id"] == a.id and not r["end"] for r in rows) and not a.force:
        sys.exit(f"{a.id}: уже есть открытая запись (--force, чтобы начать ещё одну)")
    rows.append({"id": a.id, "title": a.title or "", "start": now(), "end": "", "hours": "",
                 "commits": "", "source": "live", "note": ""})
    write(LIVE, rows)
    print(f"начало {a.id}: {rows[-1]['start']}")


def cmd_end(a):
    rows = read(LIVE)
    for r in reversed(rows):
        if r["id"] == a.id and not r["end"]:
            r["end"] = now()
            r["hours"] = f"{hours_between(r['start'], r['end']):.2f}"
            if a.note:
                r["note"] = a.note
            write(LIVE, rows)
            print(f"конец {a.id}: {r['end']}, {r['hours']} ч по часам (без вычета пауз)")
            return
    sys.exit(f"{a.id}: открытой записи нет")


def cmd_open(_):
    for r in read(LIVE):
        if not r["end"]:
            print(f"{r['id']}\t{r['start']}\t{r['title']}")


def classify(rules, subject):
    for rule in rules:
        m = re.search(rule["re"], subject, re.IGNORECASE)
        if m:
            g = m.group(1) if m.groups() and m.group(1) is not None else ""
            return rule["id"].replace("{1}", g), rule["title"].replace("{1}", g)
    return "прочее", "Не распознано правилами"


def cmd_history(a):
    rules = json.loads(RULES.read_text(encoding="utf-8"))["rules"]
    out = subprocess.run(["git", "-C", str(ROOT), "log", "--reverse", "--format=%aI|%s"],
                         capture_output=True, text=True, check=True).stdout.splitlines()
    commits = sorted(((datetime.fromisoformat(l.split("|", 1)[0]), l.split("|", 1)[1]) for l in out),
                     key=lambda c: c[0])
    stages = {}
    unknown = []
    prev = None
    for t, subject in commits:
        sid, title = classify(rules, subject)
        if sid == "прочее":
            unknown.append(subject)
        gap = 0.0 if prev is None else (t - prev).total_seconds() / 3600
        # Коммит «владеет» интервалом после предыдущего; длинный разрыв (сон, перерыв) —
        # неизвестная работа, берём условный разгон --lead.
        owned = 0.0 if prev is None else (gap if gap <= a.cap else a.lead)
        s = stages.setdefault(sid, {"title": title, "start": t, "end": t, "h": 0.0, "n": 0})
        s["start"] = min(s["start"], t - timedelta(hours=owned))
        s["end"] = max(s["end"], t)
        s["h"] += owned
        s["n"] += 1
        prev = t
    rows = [{"id": k, "title": v["title"], "start": v["start"].isoformat(timespec="seconds"),
             "end": v["end"].isoformat(timespec="seconds"), "hours": f"{v['h']:.2f}",
             "commits": v["n"], "source": "git",
             "note": f"интервал между коммитами ≤ {a.cap} ч, иначе {a.lead} ч"}
            for k, v in sorted(stages.items(), key=lambda kv: kv[1]["start"])]
    write(HIST, rows)
    total = sum(float(r["hours"]) for r in rows)
    print(f"{HIST}: этапов {len(rows)}, коммитов {len(commits)}, оценка {total:.1f} ч")
    if unknown:
        print(f"не распознано {len(unknown)}:", *unknown[:30], sep="\n  ")


def cmd_report(_):
    for path in (LIVE, HIST):
        rows = read(path)
        print(f"== {path.name} ({len(rows)})")
        for r in rows:
            print(f"{r['id']:<10} {r['hours'] or '…':>6} ч  {r['start'][:16]} → {(r['end'] or 'идёт')[:16]}  {r['title']}")


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("start"); s.add_argument("id"); s.add_argument("title", nargs="?"); s.add_argument("--force", action="store_true"); s.set_defaults(f=cmd_start)
    e = sub.add_parser("end"); e.add_argument("id"); e.add_argument("note", nargs="?"); e.set_defaults(f=cmd_end)
    sub.add_parser("open").set_defaults(f=cmd_open)
    h = sub.add_parser("history"); h.add_argument("--cap", type=float, default=2.0); h.add_argument("--lead", type=float, default=0.5); h.set_defaults(f=cmd_history)
    sub.add_parser("report").set_defaults(f=cmd_report)
    a = p.parse_args()
    a.f(a)


if __name__ == "__main__":
    main()
