#!/usr/bin/env python3
"""K-143: безголовый обход маршрутов со сбором 2D-картинок (без окна, без F10).

    python3 tools/scrape2d.py --rom ROM [--save build/full100.sav] [--lang ru] [--pack build/2d-pack-case] [routes...]

Маршрут — tools/scrape/routes/<имя>.txt: строки команд REPL sunpetal-debug; первая `goto СЦЕНА` уходит в аргумент --goto,
`@OUT@` — папка прогона. Результат: build/scrape2d/<lang>/<маршрут>/ (дампы, снимки) и отчёт: сколько ключей есть в пакете
(--pack: папка с obj*_WxH_<ключ>[@группа.вариант].png), каких нет. Без аргумента маршрутов идут все.
"""
import argparse, csv, glob, os, shutil, subprocess, sys

SCALE = int(os.environ.get('SCRAPE_SCALE', '1'))
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def pack_keys(pack):
    keys = set()
    for f in glob.glob(os.path.join(pack, '*.png')):
        n = os.path.basename(f).split('@')[0][:-4]
        if n.startswith('obj'):
            keys.add(n.split('_', 2)[2])
    return keys


def run_route(path, rom, save, lang, out, pack=None, variants=()):
    cmds, goto = [], None
    for line in open(path):
        line = line.strip()
        if not line or line.startswith('#'):
            continue
        if line.startswith('goto ') and goto is None:
            goto = line.split()[1]
        else:
            cmds.append(line.replace('@OUT@', out))
    cmds.append('quit')
    tmp = out + '.sav'
    shutil.copy(save, tmp)
    exe = os.path.join(ROOT, 'target/release/sunpetal-debug')
    args = [exe, rom, '--save', tmp, '--autoplay', '--scale', str(SCALE), '--repl', '--lang', lang]
    if goto:
        args += ['--goto', goto]
    if pack:
        args += ['--pack-2d', pack]
        for v in variants:
            args += ['--pack-variant', v]
    subprocess.run(args, input='\n'.join(cmds) + '\n', text=True, capture_output=True, check=False, timeout=600)
    os.remove(tmp)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('routes', nargs='*')
    ap.add_argument('--rom', required=True)
    ap.add_argument('--save', default='build/full100.sav')
    ap.add_argument('--lang', default='ru')
    ap.add_argument('--pack', default='build/2d-pack-case')
    ap.add_argument('--out', default='build/scrape2d')
    ap.add_argument('--variant', action='append', default=[], help='ГРУППА=ВАРИАНТ для прогона с пакетом (например digits=nunito)')
    ap.add_argument('--with-pack', action='store_true', help='ещё раз прогнать маршрут с --pack-2d: снимки *_pack.png рядом с обычными')
    a = ap.parse_args()
    names = a.routes or sorted(os.path.splitext(os.path.basename(p))[0] for p in glob.glob(os.path.join(ROOT, 'tools/scrape/routes/*.txt')))
    have = pack_keys(a.pack) if os.path.isdir(a.pack) else set()
    for name in names:
        out = os.path.join(a.out, a.lang, name)
        shutil.rmtree(out, ignore_errors=True)
        os.makedirs(out)
        run_route(os.path.join(ROOT, 'tools/scrape/routes', name + '.txt'), a.rom, a.save, a.lang, out)
        if a.with_pack:
            out2 = out + '_pack'
            shutil.rmtree(out2, ignore_errors=True)
            os.makedirs(out2)
            run_route(os.path.join(ROOT, 'tools/scrape/routes', name + '.txt'), a.rom, a.save, a.lang, out2, a.pack, a.variant)
            print(f'  с пакетом: {out2}')
        total = missing = 0
        for idx in glob.glob(os.path.join(out, '*', 'index.csv')):
            for r in csv.DictReader(open(idx)):
                if r['kind'] != 'obj':
                    continue
                total += 1
                if f"{r['key']}" not in have:
                    missing += 1
        print(f'{a.lang}/{name}: спрайтов {total}, нет в пакете {missing} ({out})')


if __name__ == '__main__':
    sys.exit(main())
