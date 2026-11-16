#!/usr/bin/env python3
"""Каталог мест (K-221, docs/AUTONOMY.md А1.4): `data/places.toml`.

    places.py list
    places.py check ROM [--save build/full100.sav] [--only a,b] [--jobs N] [--debug BIN]
    places.py where ROM ID [--save ...]            # что говорит `where` в месте (для записи expect)

`check` приводит игру в каждое место (`sunpetal-debug --repl`, копия сохранения) и сверяет `where` с `expect`.
Код возврата 1, если место не дошло или признак не совпал. `run_place` пригоден для shoot.py.
"""
import argparse
import concurrent.futures
import json
import os
import shutil
import subprocess
import sys
import tempfile
import tomllib

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..')
PLACES = os.path.join(ROOT, 'data', 'places.toml')
DEBUG = os.path.join(ROOT, 'target', 'release', 'sunpetal-debug')
SAVE = os.path.join(ROOT, 'build', 'full100.sav')


def load_places(path=PLACES):
    with open(path, 'rb') as f:
        places = tomllib.load(f)['place']
    ids = [p['id'] for p in places]
    if len(set(ids)) != len(ids):
        raise SystemExit('places: ids repeat')
    return places


def run_place(place, rom, save=SAVE, after=(), debug=DEBUG, timeout=300):
    """Ведёт игру в место и исполняет `after` (команды REPL, например `shot FILE`). Возвращает ответ `where`
    (словарь) и список ответов на `after`; на сбой запуска — ValueError."""
    with tempfile.TemporaryDirectory() as tmp:
        work = os.path.join(tmp, 'work.sav')
        shutil.copy(save, work)
        cmd = [debug, rom, '--save', work, '--scale', '1', '--repl']
        if place.get('goto'):
            cmd += ['--goto', place['goto']]
        if place.get('autoplay'):
            cmd.append('--autoplay')
        script = '\n'.join(list(place.get('steps', [])) + ['where'] + list(after) + ['quit']) + '\n'
        try:
            r = subprocess.run(cmd, input=script, capture_output=True, text=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            raise ValueError('timeout')
    replies = []
    for line in r.stdout.splitlines():
        if line.startswith('{'):
            try:
                replies.append(json.loads(line))
            except ValueError:
                pass
    # первая строка — сводка `--goto` (если он есть) или ответ первой команды
    where = next((x for x in replies if 'scene' in x and 'pos' in x), None)
    if where is None:
        raise ValueError('no answer to where; exit %s; %s' % (r.returncode, (r.stderr or r.stdout)[-300:]))
    i = replies.index(where)
    return where, replies[i + 1:]


def verdict(place, where):
    """None, если признак совпал; иначе строка с расхождением."""
    bad = []
    for k, want in place.get('expect', {}).items():
        got = where.get(k)
        if got != want:
            bad.append(f'{k}: ждали {want!r}, есть {got!r}')
    return '; '.join(bad) or None


def check(places, rom, save, jobs, debug):
    def one(p):
        try:
            w, _ = run_place(p, rom, save, debug=debug)
            return p['id'], verdict(p, w), w
        except ValueError as e:
            return p['id'], str(e), None
    with concurrent.futures.ThreadPoolExecutor(jobs) as ex:
        return list(ex.map(one, places))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest='cmd', required=True)
    sub.add_parser('list')
    for name in ('check', 'where'):
        s = sub.add_parser(name)
        s.add_argument('rom')
        if name == 'where':
            s.add_argument('id')
        s.add_argument('--save', default=SAVE)
        s.add_argument('--debug', default=DEBUG)
        if name == 'check':
            s.add_argument('--only')
            s.add_argument('--jobs', type=int, default=3)
    a = ap.parse_args()
    places = load_places()
    if a.cmd == 'list':
        for p in places:
            print(f'{p["id"]:16} {"goto " + p["goto"] if p.get("goto") else "старт":12} {p.get("desc", "")}')
        return 0
    if a.cmd == 'where':
        p = next((p for p in places if p['id'] == a.id), None)
        if not p:
            raise SystemExit(f'places: нет места {a.id}')
        print(json.dumps(run_place(p, a.rom, a.save, debug=a.debug)[0], ensure_ascii=False))
        return 0
    if a.only:
        want = set(a.only.split(','))
        unknown = want - {p['id'] for p in places}
        if unknown:
            raise SystemExit('places: нет мест ' + ', '.join(sorted(unknown)))
        places = [p for p in places if p['id'] in want]
    bad = 0
    for pid, err, w in check(places, a.rom, a.save, a.jobs, a.debug):
        if err:
            bad += 1
        print(('ПРОВАЛ ' if err else 'ok     ') + pid + (': ' + err if err else ''))
    print(f'мест {len(places)}, не прошло {bad}')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
