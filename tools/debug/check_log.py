#!/usr/bin/env python3
"""Checks a dialogue log (JSONL from `sunpetal-debug --dialogue-log`) against a scene's `.s` file and,
optionally, a table of widths: ID order, text after `<R>`/line breaks, line count, widths within a tolerance.
Prints a JSON report; exit status 1 if anything differs. Reads only what is given; stores nothing.

    check_log.py LOG.jsonl --scene C04_16 --s path/C04_16.s [--table accept-scenes.md] [--tolerance 2]

The table is the markdown of `| ID | speaker | lines | widths | text |` rows under a `## <scene>` heading.
"""
import argparse, json, re, sys


def lines_of(path):
    src = open(path, "rb").read().decode("utf-16", errors="replace").lstrip("﻿").replace("\r\n", "\n")
    out = []
    for chunk in src.split("[[")[1:]:
        head, _, body = chunk.partition("\n")
        if not head.strip().isdigit():
            continue
        out.append((head.strip(), re.sub(r"\n?<R>\n?", "\n", "\n".join(l for l in body.split("\n") if not l.lstrip().startswith("//")).strip("\n"))))
    return out


def widths_table(path, scene):
    rows, on = {}, False
    for l in open(path, encoding="utf-8"):
        if l.startswith("## "):
            on = l[3:].startswith(scene)
        elif on and l.startswith("|") and re.match(r"\|\s*\d{5}\s*\|", l):
            c = [x.strip() for x in l.strip().strip("|").split("|")]
            rows[c[0]] = [int(x) for x in c[3].split("/")]
    return rows


def main():
    a = argparse.ArgumentParser()
    a.add_argument("log")
    a.add_argument("--scene", required=True)
    a.add_argument("--s")
    a.add_argument("--table")
    a.add_argument("--tolerance", type=int, default=2)
    a = a.parse_args()
    log = [e for e in map(json.loads, open(a.log, encoding="utf-8")) if e.get("scene") == a.scene]
    problems = []
    if a.s:
        want = lines_of(a.s)
        got = [e["id"] for e in log]
        if got != [i for i, _ in want][: len(got)]:
            problems.append({"order": {"log": got, "file": [i for i, _ in want]}})
        text = dict(want)
        for e in log:
            t = text.get(e["id"])
            if t is not None and t != e["text"]:
                problems.append({"id": e["id"], "text": "differs from the file", "lines": [e["lines"], t.count("\n") + 1]})
        if len(log) < len(want):
            problems.append({"missing": [i for i, _ in want][len(log):]})
    if a.table:
        table = widths_table(a.table, a.scene)
        for e in log:
            w = table.get(e["id"])
            if w is None:
                continue
            if len(w) != e["lines"] or any(abs(x - y) > a.tolerance for x, y in zip(w, e["widths"])):
                problems.append({"id": e["id"], "widths": {"log": e["widths"], "table": w}})
    print(json.dumps({"scene": a.scene, "lines_logged": len(log), "problems": problems}, ensure_ascii=False, indent=1))
    sys.exit(1 if problems else 0)


main()
