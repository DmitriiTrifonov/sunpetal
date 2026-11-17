#!/usr/bin/env python3
"""Каталог мест (K-221, docs/AUTONOMY.md А1.4): `data/places.toml`.

    places.py list
    places.py check ROM [--save build/full100.sav] [--only a,b] [--jobs N] [--debug BIN]
    places.py where ROM ID [--save ...]            # что говорит `where` в месте (для записи expect)
    places.py replay ROM ID [--out F.png]          # закладка: повтор записи ввода безголово, контрольные точки, кадр

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
# закладки автора (консоль окна `bookmark ИМЯ`, K-221 А1.6): не в git, дописываются в этот файл
BOOKMARKS = os.path.join(ROOT, 'artifacts', 'bookmarks', 'places.toml')
HEADLESS = os.path.join(ROOT, 'target', 'release', 'sunpetal-headless')
DEBUG = os.path.join(ROOT, 'target', 'release', 'sunpetal-debug')
SAVE = os.path.join(ROOT, 'build', 'full100.sav')


def load_places(path=PLACES, bookmarks=BOOKMARKS):
    with open(path, 'rb') as f:
        places = tomllib.load(f)['place']
    if bookmarks and os.path.exists(bookmarks):
        with open(bookmarks, 'rb') as f:
            places += tomllib.load(f).get('place', [])
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


def run_replay(place, rom, out=None, headless=HEADLESS, timeout=900):
    """Закладка (`replay` — папка с input.txt, start.sav, input.chk): повтор ввода безголово до кадра `frames`, сверка
    контрольных точек записи (`--check`), кадр на `frames` в PNG. Возвращает (число сошедшихся точек, путь PNG);
    расхождение или сбой — ValueError."""
    d = place['replay']
    if not os.path.isabs(d):
        d = os.path.join(ROOT, d)
    frames = int(place['frames'])
    with tempfile.TemporaryDirectory() as tmp:
        work = os.path.join(tmp, 'work.sav')
        shutil.copy(os.path.join(d, 'start.sav'), work)
        with open(os.path.join(d, 'input.txt')) as f:
            inp = f.read().strip().rstrip(',')
        # контрольные точки только до кадра закладки
        chk = os.path.join(tmp, 'input.chk')
        with open(os.path.join(d, 'input.chk')) as f, open(chk, 'w') as g:
            g.writelines(l for l in f if l.split(' ')[0].isdigit() and int(l.split(' ')[0]) <= frames)
        cmd = [headless, rom, '--save', work, '--input', inp, '--vblanks', str(frames), '--shots', str(frames), '--out', tmp, '--check', chk]
        try:
            r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            raise ValueError('timeout')
        if r.returncode != 0:
            raise ValueError('replay: код %s; %s' % (r.returncode, (r.stderr or r.stdout)[-300:]))
        ok = r.stderr.count('checkpoint ') - r.stderr.count('diverged')
        png = os.path.join(tmp, 'v%d.png' % frames)
        if out and os.path.exists(png):
            shutil.copy(png, out)
        else:
            out = None
    return ok, out


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
        if p.get('replay'):
            try:
                n, _ = run_replay(p, rom)
                return p['id'], None, {'checkpoints': n}
            except ValueError as e:
                return p['id'], str(e), None
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
    r = sub.add_parser('replay')
    r.add_argument('rom')
    r.add_argument('id')
    r.add_argument('--out')
    r.add_argument('--headless', default=HEADLESS)
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
            how = 'закладка' if p.get('replay') else ('goto ' + p['goto'] if p.get('goto') else 'старт')
            print(f'{p["id"]:16} {how:12} {p.get("desc", "")}')
        return 0
    if a.cmd == 'replay':
        p = next((p for p in places if p['id'] == a.id and p.get('replay')), None)
        if not p:
            raise SystemExit(f'places: нет закладки {a.id}')
        try:
            ok, png = run_replay(p, a.rom, a.out, a.headless)
        except ValueError as e:
            print('ПРОВАЛ ' + str(e))
            return 1
        print(f'ok: кадр {p["frames"]}, контрольных точек сошлось {ok}' + (f', {png}' if png else ''))
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
