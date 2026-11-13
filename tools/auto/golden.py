#!/usr/bin/env python3
"""Золотые кадры (К-225, docs/AUTONOMY.md А2.6): обход мест `data/golden/places.toml` без окна и без модели,
хэши кадров против базы, отчёт «что изменилось» с парами картинок.

    golden.py run ROM SAVE [--base DIR] [--out DIR] [--only a,b] [--jobs N] [--debug BIN]
    golden.py accept [--out DIR] [--base DIR] [--only a,b]     # новый прогон становится базой
    golden.py compare [--out DIR] [--base DIR]                  # только сравнение и отчёт по готовым кадрам

`run` снимает каждое место на копии сохранения (`sunpetal-debug --repl`, кадр обоих экранов, масштаб 1) в OUT
(по умолчанию `artifacts/golden/new`), затем сравнивает с BASE (`artifacts/golden/base`) и пишет `report.md` и
`report.html` в `OUT/report/`: у изменившегося места — база, новый кадр и карта различий. Код возврата 1, если
что-то изменилось или места нет в базе; 2 — ошибка запуска. Переменные окружения (`SUNPETAL_*`) уходят в
игру как есть: так снимается «до» и «после» правки. Базы и кадры в git не идут (`artifacts/`).
"""
import concurrent.futures
import hashlib
import html
import os
import shutil
import subprocess
import sys
import tempfile
import tomllib

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..')
sys.path.insert(0, os.path.join(ROOT, 'tools', 'ps2'))
import lf

PLACES = os.path.join(ROOT, 'data', 'golden', 'places.toml')
DEBUG = os.path.join(ROOT, 'target', 'release', 'sunpetal-debug')


def load_places(only=None):
    with open(PLACES, 'rb') as f:
        places = tomllib.load(f)['place']
    names = [p['name'] for p in places]
    if len(set(names)) != len(names):
        raise SystemExit('golden: names of places repeat')
    if only:
        missing = set(only) - set(names)
        if missing:
            raise SystemExit(f'golden: no such place: {", ".join(sorted(missing))}')
        places = [p for p in places if p['name'] in only]
    return places


def shoot(place, rom, save, out, debug=DEBUG):
    """Один кадр места: PNG в `out/<name>.png`. Возвращает (name, ошибка или None)."""
    name = place['name']
    target = os.path.join(out, name + '.png')
    with tempfile.TemporaryDirectory() as tmp:
        work = os.path.join(tmp, 'work.sav')
        shutil.copy(save, work)
        cmd = [debug, rom, '--save', work, '--goto', place['goto'], '--scale', '1', '--repl']
        if place.get('autoplay'):
            cmd.append('--autoplay')
        script = '\n'.join(place.get('steps', []) + [f'shot {target}', 'quit']) + '\n'
        try:
            r = subprocess.run(cmd, input=script, capture_output=True, text=True, timeout=300)
        except subprocess.TimeoutExpired:
            return name, 'timeout 300 s'
    if not os.path.isfile(target):
        tail = (r.stdout.strip().splitlines() or r.stderr.strip().splitlines() or ['no output'])[-1]
        return name, f'no shot: {tail[:160]}'
    return name, None


def pixels(path):
    w, h, rgba = lf.read_png(path)
    return w, h, bytes(rgba)


def digest(path):
    w, h, px = pixels(path)
    return hashlib.sha256(w.to_bytes(4, 'big') + h.to_bytes(4, 'big') + px).hexdigest()


def diff(a_path, b_path):
    """Сравнение двух кадров: None, если равны; иначе словарь со счётчиками и картой различий (w, h, rgba)."""
    wa, ha, a = pixels(a_path)
    wb, hb, b = pixels(b_path)
    if (wa, ha) != (wb, hb):
        return {'size': ((wa, ha), (wb, hb)), 'changed': None, 'max': None, 'bbox': None, 'map': None}
    if a == b:
        return None
    n, mx = 0, 0
    x0, y0, x1, y1 = wa, ha, -1, -1
    m = bytearray(wa * ha * 4)
    for i in range(0, len(a), 4):
        d = max(abs(a[i + k] - b[i + k]) for k in range(4))
        if d:
            n += 1
            mx = max(mx, d)
            x, y = divmod(i // 4, wa)[::-1]
            x0, y0, x1, y1 = min(x0, x), min(y0, y), max(x1, x), max(y1, y)
            m[i:i + 4] = bytes((255, 255 - min(255, d * 4), 0, 255))
        else:
            g = (a[i] + a[i + 1] + a[i + 2]) // 3 // 3 + 40
            m[i:i + 4] = bytes((g, g, g, 255))
    return {'size': None, 'changed': n, 'max': mx, 'bbox': (x0, y0, x1, y1), 'map': (wa, ha, bytes(m))}


def compare(out, base, names):
    """[(name, 'same' | 'changed' | 'new' | 'missing', info)] по местам из `names`."""
    rows = []
    for name in names:
        new, old = os.path.join(out, name + '.png'), os.path.join(base, name + '.png')
        if not os.path.isfile(new):
            rows.append((name, 'missing', None))
        elif not os.path.isfile(old):
            rows.append((name, 'new', None))
        else:
            d = diff(old, new)
            rows.append((name, 'same' if d is None else 'changed', d))
    return rows


def report(out, base, rows):
    """Пишет OUT/report/report.md и report.html с картами различий; возвращает путь к report.md."""
    rd = os.path.join(out, 'report')
    os.makedirs(rd, exist_ok=True)
    md = ['# Золотые кадры: что изменилось', '']
    cnt = {k: sum(1 for r in rows if r[1] == k) for k in ('same', 'changed', 'new', 'missing')}
    md.append(f'Мест {len(rows)}: без изменений {cnt["same"]}, изменилось {cnt["changed"]}, нет в базе {cnt["new"]}, '
              f'не снято {cnt["missing"]}.')
    md.append('')
    cards = []
    for name, st, d in rows:
        if st == 'same':
            continue
        if st == 'changed' and d['map']:
            w, h, m = d['map']
            lf.write_png(os.path.join(rd, name + '_diff.png'), w, h, m)
            shutil.copy(os.path.join(base, name + '.png'), os.path.join(rd, name + '_base.png'))
            shutil.copy(os.path.join(out, name + '.png'), os.path.join(rd, name + '_new.png'))
            info = (f'{d["changed"]} пикселей, наибольшая разность канала {d["max"]}, '
                    f'рамка {d["bbox"][0]},{d["bbox"][1]}–{d["bbox"][2]},{d["bbox"][3]}')
            md.append(f'- **{name}**: изменился — {info}')
            cards.append(f'<section><h2>{html.escape(name)}</h2><p>{html.escape(info)}</p><div class="row">'
                         f'<figure><img src="{name}_base.png"><figcaption>база</figcaption></figure>'
                         f'<figure><img src="{name}_new.png"><figcaption>сейчас</figcaption></figure>'
                         f'<figure><img src="{name}_diff.png"><figcaption>различия</figcaption></figure></div></section>')
        elif st == 'changed':
            info = f'размер {d["size"][0]} → {d["size"][1]}'
            md.append(f'- **{name}**: изменился — {info}')
            cards.append(f'<section><h2>{html.escape(name)}</h2><p>{html.escape(info)}</p></section>')
        elif st == 'new':
            md.append(f'- **{name}**: нет в базе (принять: `golden.py accept`)')
            cards.append(f'<section><h2>{html.escape(name)}</h2><p>нет в базе</p></section>')
        else:
            md.append(f'- **{name}**: кадр не снят')
            cards.append(f'<section><h2>{html.escape(name)}</h2><p>кадр не снят</p></section>')
    path = os.path.join(rd, 'report.md')
    open(path, 'w', encoding='utf-8').write('\n'.join(md) + '\n')
    page = ('<!doctype html><meta charset="utf-8"><title>Золотые кадры</title>'
            '<style>body{font:15px system-ui;margin:16px;background:#fafafa;color:#222}'
            '.row{display:flex;gap:12px;flex-wrap:wrap}figure{margin:0}img{max-width:420px;image-rendering:pixelated;'
            'border:1px solid #ccc}</style>' + f'<h1>Золотые кадры</h1><p>{html.escape(md[2])}</p>' + ''.join(cards))
    open(os.path.join(rd, 'report.html'), 'w', encoding='utf-8').write(page)
    return path


def parse(argv):
    opts = {'base': os.path.join(ROOT, 'artifacts', 'golden', 'base'), 'out': os.path.join(ROOT, 'artifacts', 'golden', 'new'),
            'only': None, 'jobs': 3, 'debug': DEBUG}
    pos, i = [], 0
    while i < len(argv):
        a = argv[i]
        if a in ('--base', '--out', '--only', '--jobs', '--debug'):
            v = argv[i + 1]
            opts[a[2:]] = v.split(',') if a == '--only' else int(v) if a == '--jobs' else os.path.abspath(v)
            i += 2
        else:
            pos.append(a)
            i += 1
    return pos, opts


def main(argv):
    pos, o = parse(argv)
    if not pos:
        print(__doc__)
        return 2
    cmd = pos[0]
    names = [p['name'] for p in load_places(o['only'])]
    if cmd == 'accept':
        os.makedirs(o['base'], exist_ok=True)
        for n in names:
            src = os.path.join(o['out'], n + '.png')
            if os.path.isfile(src):
                shutil.copy(src, os.path.join(o['base'], n + '.png'))
        print(f'accepted {len(names)} places into {o["base"]}')
        return 0
    if cmd == 'run':
        if len(pos) != 3:
            print(__doc__)
            return 2
        rom, save = os.path.abspath(pos[1]), os.path.abspath(pos[2])
        os.makedirs(o['out'], exist_ok=True)
        for n in names:
            p = os.path.join(o['out'], n + '.png')
            if os.path.isfile(p):
                os.remove(p)
        errs = {}
        with concurrent.futures.ThreadPoolExecutor(o['jobs']) as ex:
            futs = [ex.submit(shoot, p, rom, save, o['out'], o['debug']) for p in load_places(o['only'])]
            for f in concurrent.futures.as_completed(futs):
                n, e = f.result()
                print(f'{n}: {e or "ok"}', flush=True)
                if e:
                    errs[n] = e
        if len(errs) == len(names):
            return 2
    elif cmd != 'compare':
        print(__doc__)
        return 2
    rows = compare(o['out'], o['base'], names)
    path = report(o['out'], o['base'], rows)
    print(open(path, encoding='utf-8').read())
    return 0 if all(r[1] == 'same' for r in rows) else 1


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
