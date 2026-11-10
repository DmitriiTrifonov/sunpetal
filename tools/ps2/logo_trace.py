#!/usr/bin/env python3
"""Векторный апскейл плоских логотипов (K-196): квантование → маска цвета → potrace → слои.

    logo_trace.py in.png out.png --colors K [--scale 4] [--smooth 1.0]

Нужны magick (ImageMagick) и potrace. Для логотипов с градиентами/свечением не годится.
"""
import argparse
import os
import subprocess
import sys
import tempfile


def run(*a):
    return subprocess.run(a, check=True, capture_output=True, text=True).stdout


def palette(src, k):
    out = run('magick', src, '-colors', str(k), '+dither', '-unique-colors', '-depth', '8', 'txt:-')
    cols = []
    for ln in out.splitlines()[1:]:
        cols.append(ln.split('#')[1][:6])
    return cols


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument('src')
    ap.add_argument('dst')
    ap.add_argument('--colors', type=int, default=4)
    ap.add_argument('--palette', help='цвета через запятую (hex без #): ближайший цвет вместо квантования')
    ap.add_argument('--soft', action='store_true', help='маски по проекции на линию фон→цвет (края текста без «чужих» пятен)')
    ap.add_argument('--scale', type=int, default=4)
    ap.add_argument('--blur', type=float, default=0.4, help='размытие маски перед порогом, в долях scale')
    ap.add_argument('--smooth', type=float, default=1.0)
    a = ap.parse_args(argv)
    w, h = map(int, run('magick', 'identify', '-format', '%w %h', a.src).split())
    W, H = w * a.scale, h * a.scale
    t = tempfile.mkdtemp()
    q = os.path.join(t, 'q.png')
    if a.palette:
        cols = a.palette.split(',')
        pal = os.path.join(t, 'pal.png')
        run('magick', *['xc:#' + c for c in cols], '+append', '-type', 'TrueColor', 'PNG24:' + pal)
        run('magick', a.src, '-alpha', 'off', '-type', 'TrueColor', '-remap', pal, '+dither', '-type', 'TrueColor', 'PNG24:' + q)
    else:
        run('magick', a.src, '-colors', str(a.colors), '+dither', q)
        cols = palette(q, a.colors)
    # площадь каждого цвета
    areas = []
    bgc = None
    for c in cols:
        m = os.path.join(t, 'm_%s.pgm' % c)
        if a.soft:
            bgc = cols[0]  # первый цвет палитры = фон
            if c == bgc:
                run('magick', q, '-fill', 'black', '+opaque', '#' + c, '-fill', 'white', '-opaque', '#' + c, '-colorspace', 'Gray', m)
            else:
                B = [int(bgc[i:i + 2], 16) / 255 for i in (0, 2, 4)]
                C = [int(c[i:i + 2], 16) / 255 for i in (0, 2, 4)]
                D = [C[i] - B[i] for i in range(3)]
                dd = sum(x * x for x in D)
                dot = '((u.r-%f)*%f+(u.g-%f)*%f+(u.b-%f)*%f)/%f' % (B[0], D[0], B[1], D[1], B[2], D[2], dd)
                tt = '(' + dot + ')'
                res = 'sqrt(((u.r-%f)-%s*%f)^2+((u.g-%f)-%s*%f)^2+((u.b-%f)-%s*%f)^2)' % (
                    B[0], tt, D[0], B[1], tt, D[1], B[2], tt, D[2])
                fx = 'min(1,max(0,%s))*min(1,max(0,1-%s/0.22))' % (tt, res)
                run('magick', a.src, '-alpha', 'off', '-colorspace', 'sRGB', '-type', 'TrueColor', '-fx', fx, '-colorspace', 'Gray', m)
            mean = float(run('magick', m, '-format', '%[fx:mean]', 'info:'))
            areas.append((mean, c, m))
            continue
        run('magick', q, '-fill', 'black', '+opaque', '#' + c, '-fill', 'white', '-opaque', '#' + c, '-colorspace', 'Gray', m)
        mean = float(run('magick', m, '-format', '%[fx:mean]', 'info:'))
        areas.append((mean, c, m))
    areas.sort(reverse=True)
    # фон = самый крупный цвет
    run('magick', '-size', '%dx%d' % (W, H), 'xc:#' + areas[0][1], '-type', 'TrueColor', 'PNG24:' + os.path.join(t, 'out.png'))
    for mean, c, m in areas[1:]:
        big = os.path.join(t, 'b_%s.pbm' % c)
        run('magick', m, '-filter', 'Lanczos', '-resize', '%dx%d!' % (W, H), '-blur', '0x%g' % (a.scale * a.blur),
            '-threshold', '50%', '-negate', big)
        lay = os.path.join(t, 'l_%s.pgm' % c)
        # potrace рисует чёрное по белому: 1 = нужный цвет
        run('potrace', '-b', 'pgm', '-r', '72', '-W', '%dpt' % W, '-H', '%dpt' % H, '-a', str(a.smooth), '-O', '0.4',
            '-t', '4', '-o', lay, big)
        col = os.path.join(t, 'c_%s.png' % c)
        run('magick', '-size', '%dx%d' % (W, H), 'xc:#' + c, '-colorspace', 'sRGB', '-type', 'TrueColor', '(', lay, '-negate', ')',
            '-alpha', 'off', '-compose', 'CopyOpacity', '-composite', 'PNG32:' + col)
        run('magick', os.path.join(t, 'out.png'), col, '-compose', 'Over', '-composite', '-type', 'TrueColor',
            'PNG24:' + os.path.join(t, 'out2.png'))
        os.replace(os.path.join(t, 'out2.png'), os.path.join(t, 'out.png'))
    if os.environ.get('LOGO_KEEP'):
        print(t)
    os.replace(os.path.join(t, 'out.png'), a.dst)


if __name__ == '__main__':
    main(sys.argv[1:])
