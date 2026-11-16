#!/usr/bin/env python3
"""Один вход для снимка места (K-221, docs/AUTONOMY.md А1.5): `data/places.toml`.

    shoot.py ROM МЕСТО --out FILE.png [--save build/full100.sav] [--set КЛЮЧ=ЗНАЧЕНИЕ ...] [--app] [--size W H]

Без `--app` снимок экранов DS через `sunpetal-debug` (масштаб 1, оба экрана). С `--app` — кадр окна приложения
(`sunpetal-app --script`: раскладки, чёткий текст, эффекты); команды места переводятся на язык скрипта приложения
(`advance N` -> `wait N`, `skip` -> `until dialogue`, `until idle`, `press`, `tap` как есть; иное — отказ). `--set`: без `--app` —
переменная окружения (`SUNPETAL_X=1` или `X=1` -> `SUNPETAL_X=1`), с `--app` — строка `set путь значение`
(`video.layout=single`). Печатает признак места (`where`, без `--app`) и путь к кадру.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import places

APP = os.path.join(places.ROOT, 'target', 'release', 'sunpetal-app')


def app_script(place, sets, out, size):
    lines = ['size %d %d 1' % tuple(size)]
    for kv in sets:
        k, _, v = kv.partition('=')
        lines.append(f'set {k} {v}')
    for st in place.get('steps', []):
        w = st.split()
        if w[0] == 'advance':
            lines.append('wait ' + ' '.join(w[1:]))
        elif w[0] == 'skip':
            # после --goto приложение сперва в диалоге, потом в ходьбе: ждём оба (до 3000 кадров)
            lines += ['until dialogue 3000', 'until idle 3000']
        elif w[0] in ('press', 'tap'):
            lines.append(st)
        else:
            raise SystemExit(f'shoot: команда «{st}» места не переводится на скрипт приложения')
    lines += [f'shot {out}', 'quit']
    return '\n'.join(lines) + '\n'


def shoot_app(place, rom, save, out, sets, size, timeout=600):
    with tempfile.TemporaryDirectory() as tmp:
        work = os.path.join(tmp, 'work.sav')
        shutil.copy(save, work)
        cmd = [APP, rom, '--save', work, '--script', '-']
        if place.get('goto'):
            cmd += ['--goto', place['goto']]
        r = subprocess.run(cmd, input=app_script(place, sets, out, size), capture_output=True, text=True, timeout=timeout)
    if not os.path.isfile(out):
        raise SystemExit('shoot: кадра нет; %s' % (r.stderr or r.stdout)[-400:])


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('rom')
    ap.add_argument('place')
    ap.add_argument('--out', required=True)
    ap.add_argument('--save', default=places.SAVE)
    ap.add_argument('--set', action='append', default=[], dest='sets')
    ap.add_argument('--app', action='store_true')
    ap.add_argument('--size', nargs=2, type=int, default=[1280, 720])
    ap.add_argument('--debug', default=places.DEBUG)
    a = ap.parse_args()
    place = next((p for p in places.load_places() if p['id'] == a.place), None)
    if not place:
        raise SystemExit(f'shoot: нет места {a.place} (places.py list)')
    out = os.path.abspath(a.out)
    os.makedirs(os.path.dirname(out), exist_ok=True)
    if a.app:
        shoot_app(place, a.rom, a.save, out, a.sets, a.size)
        print(out)
        return 0
    for kv in a.sets:
        k, _, v = kv.partition('=')
        os.environ[k if k.startswith('SUNPETAL_') else 'SUNPETAL_' + k] = v
    where, _ = places.run_place(place, a.rom, a.save, after=[f'shot {out}'], debug=a.debug)
    err = places.verdict(place, where)
    if not os.path.isfile(out):
        raise SystemExit('shoot: кадра нет')
    print(json.dumps(where, ensure_ascii=False))
    print(out)
    if err:
        print('shoot: признак места не совпал: ' + err, file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
