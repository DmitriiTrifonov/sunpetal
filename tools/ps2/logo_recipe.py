#!/usr/bin/env python3
"""K-196: рецепт стартовых логотипов (доводка поверх `logo_pack.py`).

    logo_recipe.py --base build/2d-pack-case --out build/2d-pack-case \
                   [--serif-font FILE] [--sans-font FILE] [--rising-png FILE|--no-download]

В репозиторий и в раздачу идёт рецепт, не картинки: слои собираются на машине пользователя из его ROM
(`logo_pack.py` → увеличенные `bgNN_<ключ>.png` 1024×768) плюс шаги ниже. Нужны `magick` и `potrace`.

  Rising Star Games  официальный PNG издателя (скачивается сюда, sha256 печатается; не скачался — трассировка по базе)
  герб Grasshopper   трассировка по базе (`logo_trace.py --scale 2 --blur 0.25`) + «Video Game Band» заново (serif-шрифт, дуга)
  Marvelous          трассировка по базе + надпись заново (sans-шрифт)
  Act Imagine, h.a.n.d.  остаются как в базе
Шрифты: свои пути или открытые по умолчанию (Liberation Sans Bold, EB Garamond, DejaVu Serif); нет шрифта — надпись остаётся трассировкой.
Ключи слоёв — как в ROM EUR (`YHNP`). Координаты рамок — на сетке 1024×768.
"""
import argparse
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
RISING = 'bg02_340c414d7a2192b1.png'
MARVEL = 'bg02_c85fa7e0936f75ae.png'
CREST = 'bg10_9afb33b0ff993569.png'
RISING_URL = 'https://cdn.xn--h1ajim.xn--p1ai/images/3/3e/Rising_Star_Games_logo.png'
SANS = ['/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf', '/Library/Fonts/LiberationSans-Bold.ttf',
        '/usr/share/fonts/liberation-sans/LiberationSans-Bold.ttf', '/System/Library/Fonts/Supplemental/Arial Bold.ttf']
SERIF = ['/usr/share/fonts/truetype/ebgaramond/EBGaramond-Regular.ttf', '/usr/share/fonts/opentype/ebgaramond/EBGaramond12-Regular.otf',
         '/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf', '/System/Library/Fonts/Supplemental/Hoefler Text.ttc']


def run(*a):
    return subprocess.run(a, check=True, capture_output=True, text=True).stdout


def pick(arg, cands):
    for f in ([arg] if arg else []) + cands:
        if f and os.path.exists(f):
            return f
    return None


def trace(src, dst, *opts):
    run(sys.executable, os.path.join(HERE, 'logo_trace.py'), src, dst, *opts)


def rising(base, out, png, download):
    if png is None and download:
        try:
            png = os.path.join(tempfile.mkdtemp(), 'rsg.png')
            urllib.request.urlretrieve(RISING_URL, png)
            print('Rising: скачан', RISING_URL, 'sha256', hashlib.sha256(open(png, 'rb').read()).hexdigest())
        except Exception as e:
            print('Rising: не скачался (%s), трассировка по базе' % e)
            png = None
    if png is None:
        trace(os.path.join(base, RISING), os.path.join(out, RISING), '--palette', 'FBFBFB,EB3030,101010', '--soft', '--scale', '1')
        return
    run('magick', png, '-fuzz', '4%', '-fill', '#FBFBFB', '-opaque', '#FFFFFF', '-fuzz', '1%', '-fill', '#FBFBFB', '-opaque', '#EEEEEE',
        '-fuzz', '6%', '-fill', '#EB3030', '-opaque', '#EF4B48', '-fuzz', '8%', '-fill', '#101010', '-opaque', '#231F20',
        '-filter', 'Lanczos', '-resize', '579x634!', os.path.join(os.path.dirname(png), 'r.png'))
    run('magick', '-size', '1024x768', 'xc:#FBFBFB', '-type', 'TrueColor', os.path.join(os.path.dirname(png), 'r.png'),
        '-geometry', '+220+65', '-composite', 'PNG24:' + os.path.join(out, RISING))


def crest(base, out, font):
    tmp = tempfile.mkdtemp()
    t = os.path.join(tmp, 't.png')
    trace(os.path.join(base, CREST), t, '--palette', '8AC330,202869,FBFBFB', '--scale', '2', '--blur', '0.25', '--smooth', '0.6')
    run('magick', t, '-filter', 'Lanczos', '-resize', '1024x768', 'PNG24:' + t)
    if not font:
        print('герб: нет serif-шрифта, «Video Game Band» остаётся трассировкой')
        shutil.copy(t, os.path.join(out, CREST))
        return
    run('magick', t, '-fill', '#8AC330', '-draw',
        'polygon 345,664 358,664 370,667 450,673 530,676 610,672 690,666 692,700 650,716 610,722 530,724 450,721 410,716 370,705 345,692',
        os.path.join(tmp, 'e.png'))
    run('magick', '-background', 'none', '-font', font, '-pointsize', '80', '-fill', '#202869', '-stroke', '#202869', '-strokewidth', '1.6',
        'label:Video Game Band', '-trim', '+repage', '-resize', '307x', '-rotate', '180', '-background', 'none',
        '-virtual-pixel', 'background', '-distort', 'Arc', '30', '-rotate', '180', '-trim', '+repage', os.path.join(tmp, 'w.png'))
    run('magick', os.path.join(tmp, 'e.png'), os.path.join(tmp, 'w.png'), '-geometry', '+356+662', '-composite',
        'PNG24:' + os.path.join(out, CREST))


def marvelous(base, out, font):
    tmp = tempfile.mkdtemp()
    t = os.path.join(tmp, 't.png')
    trace(os.path.join(base, MARVEL), t, '--palette', 'FBFBFB,080808', '--scale', '1', '--soft')
    if not font:
        print('Marvelous: нет sans-шрифта, надпись остаётся трассировкой')
        shutil.copy(t, os.path.join(out, MARVEL))
        return
    run('magick', t, '-fill', '#FBFBFB', '-draw', 'rectangle 118,458 905,532', os.path.join(tmp, 'e.png'))
    run('magick', '-background', 'none', '-font', font, '-pointsize', '100', '-kerning', '-3', '-fill', '#080808',
        'label:Marvelous Entertainment Inc.', '-trim', '+repage', '-resize', '762x', os.path.join(tmp, 'w.png'))
    run('magick', os.path.join(tmp, 'e.png'), os.path.join(tmp, 'w.png'), '-geometry', '+130+466', '-composite',
        'PNG24:' + os.path.join(out, MARVEL))


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument('--base', required=True, help='папка с увеличенными слоями (вывод logo_pack.py)')
    ap.add_argument('--out', required=True)
    ap.add_argument('--serif-font')
    ap.add_argument('--sans-font')
    ap.add_argument('--rising-png', help='готовый файл логотипа вместо скачивания')
    ap.add_argument('--no-download', action='store_true')
    a = ap.parse_args(argv)
    os.makedirs(a.out, exist_ok=True)
    if os.path.abspath(a.base) == os.path.abspath(a.out):
        # базу читаем до записи: копия
        b = tempfile.mkdtemp()
        for k in (RISING, MARVEL, CREST):
            shutil.copy(os.path.join(a.base, k), os.path.join(b, k))
        a.base = b
    serif, sans = pick(a.serif_font, SERIF), pick(a.sans_font, SANS)
    print('шрифты: serif', serif, '| sans', sans)
    rising(a.base, a.out, a.rising_png, not a.no_download)
    crest(a.base, a.out, serif)
    marvelous(a.base, a.out, sans)


if __name__ == '__main__':
    main(sys.argv[1:])
