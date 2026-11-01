#!/usr/bin/env python3
"""Style guide of the case redraws (docs/STYLE-2D.md): re-measures the palette from the PS2 pictures,
prints the table and draws the test tile.

    python3 tools/style-swatches.py [--measure] [--tile]      (both by default)

Needs only the standard library and ImageMagick (`magick`). Reads `build/ps2-case/png/` (made by
tools/ps2/case.py, not in git) and `artifacts/menu8g/four.png` (the DS screens); writes
`artifacts/2d-samples/style/` (not in git): `palette.txt`, `tile.png`, icon SVGs and sheets.
Colours are read from the pixels: `mean` is the average of the box, `domN` the N-th most common colour
of the box after reduction to 6 colours (the boxes hold lit and shaded parts, so a mean would be mud).
The boxes are in the pixels of the original picture and were picked by eye on a x2 view.
"""
import os, re, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PNG = os.path.join(ROOT, 'build/ps2-case/png')
OUT = os.path.join(ROOT, 'artifacts/2d-samples/style')
FONT = os.path.join(ROOT, 'build/fonts/firasanscondensed/FiraSansCondensed-Medium.ttf')
R = '016_006_352x420_T8.png'  # the render of the open case, 352x420
# name, picture, box x,y,w,h, mode, what it is
REGIONS = [
    ('panel-mid',      R, (303, 85, 20, 65),  'dom0', 'зелёная панель крышки, середина тона'),
    ('panel-stroke',   R, (303, 85, 20, 65),  'dom1', 'жёлтые дуги-штрихи в углах панели'),
    ('panel-light',    R, (303, 85, 20, 65),  'dom2', 'зелёная панель, свет'),
    ('panel-shadow',   R, (303, 85, 20, 65),  'dom3', 'тень у кромки панели, линии'),
    ('base-green',     R, (104, 215, 190, 25), 'dom0', 'зелёная плата основания (приборная полоса), тон'),
    ('base-green-dk',  R, (104, 215, 190, 25), 'dom1', 'та же плата, тень'),
    ('silver-mid',     R, (185, 90, 70, 70),  'dom0', 'серебро диска, середина'),
    ('silver-light',   R, (185, 90, 70, 70),  'dom1', 'серебро диска, свет'),
    ('silver-high',    R, (185, 90, 70, 70),  'dom3', 'серебро диска, блик'),
    ('silver-shade',   R, (185, 90, 70, 70),  'dom5', 'серебро диска, тень'),
    ('engraving',      R, (185, 90, 70, 70),  'dom2', 'чёрный: гравировка, кольцо вокруг диска, ручка'),
    ('rim-light',      R, (150, 50, 6, 4),    'mean', 'рамка чемодана, верх (на свету)'),
    ('rim-shade',      R, (30, 340, 120, 10), 'dom1', 'рамка чемодана, низ (в тени)'),
    ('bordeaux',       R, (125, 82, 24, 100), 'dom0', 'бордо полосы кнопок'),
    ('bordeaux-lt',    R, (125, 82, 24, 100), 'dom4', 'бордо, светлая кромка'),
    ('button-plate',   R, (125, 82, 24, 100), 'dom2', 'светлая плитка кнопки'),
    ('gold',           R, (125, 82, 24, 100), 'dom5', 'золото значков, уголки рамки'),
    ('title-gold',     R, (118, 56, 32, 12),  'dom0', 'золото заголовка KATHARINE на панели'),
    ('felt',           R, (60, 300, 40, 12),  'mean', 'тёмная подложка основания (синий почти чёрный)'),
    ('lamp-red',       R, (308, 163, 5, 5),   'mean', 'красная лампа и линия'),
    ('hint-green',     '016_122_392x112_T4.png', (0, 0, 392, 112), 'dom1', 'HINT!: ярко-зелёный (в игре метка красится вершинно)'),
    ('frame-grey',     '016_114_392x112_T4.png', (0, 0, 392, 112), 'dom1', 'рамка KATHARINE в картинке: серый (в игре красится)'),
]
DS = [  # the DS screens (artifacts/menu8g/four.png, the lower half starts at y=192)
    ('ds-panel',   (50, 20, 6, 6),    'mean', 'зелёная панель DS (верхний экран)'),
    ('ds-bar',     (20, 170 + 192, 10, 6), 'mean', 'нижняя зелёная полоса DS'),
    ('ds-navy',    (10, 110 + 192, 8, 8), 'mean', 'тёмно-синяя плита DS, середина'),
    ('ds-navy-edge', (12, 42 + 192, 6, 6), 'mean', 'тёмно-синяя плита DS, край (виньетка)'),
]


def run(*a):
    return subprocess.check_output(['magick', *map(str, a)]).decode()


def probe(img, box, mode):
    x, y, w, h = box
    base = [img, '-alpha', 'off', '-crop', f'{w}x{h}+{x}+{y}', '+repage']
    if mode == 'mean':
        return '#' + run(*base, '-scale', '1x1!', '-format', '%[hex:u.p{0,0}]', 'info:')
    n = int(mode[3:])
    hist = run(*base, '-colors', 6, '-format', '%c', 'histogram:info:-').splitlines()
    rows = sorted(((int(m[1]), m[2]) for m in (re.match(r'\s*(\d+):.*?(#[0-9A-Fa-f]{6})', l) for l in hist) if m), reverse=True)
    return rows[min(n, len(rows) - 1)][1].upper()


def measure():
    os.makedirs(OUT, exist_ok=True)
    lines = []
    for name, f, box, mode, what in REGIONS:
        lines.append((name, probe(os.path.join(PNG, f), box, mode), f, box, mode, what))
    four = os.path.join(ROOT, 'artifacts/menu8g/four.png')
    for name, box, mode, what in DS:
        lines.append((name, probe(four, box, mode), 'four.png', box, mode, what))
    with open(os.path.join(OUT, 'palette.txt'), 'w') as o:
        for n, c, f, b, m, w in lines:
            o.write(f'{n}\t{c}\t{f}\t{b[0]},{b[1]},{b[2]},{b[3]}\t{m}\t{w}\n')
            print(f'{n:14} {c}  {f[:12]:12} {b} {m:5} {w}')
    return {n: c for n, c, *_ in lines}


# ------------------------------------------------------------------ the tile
def svg(w, h, body):
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}">{body}</svg>'


def rr(x, y, w, h, f, r=0):
    return f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{f}"/>'


def ci(x, y, r, f):
    return f'<circle cx="{x}" cy="{y}" r="{r}" fill="{f}"/>'


def poly(p, f):
    return f'<polygon points="{p}" fill="{f}"/>'


# the PS2 case as a rule: a plate has a lit edge on the upper left, a shaded one on the lower right
# (1 DS pixel each), a mid face, a black engraving; the accents are the panel's green with the yellow
# strokes and the bordeaux of the button strip.
def plate(x, y, w, h, r, face, light, shade):
    return rr(x, y, w, h, shade, r) + rr(x, y, w - 1.1, h - 1.1, light, r) + rr(x + 1.1, y + 1.1, w - 2.2, h - 2.2, face, max(r - 1, 0))


def icons(p):
    K, SL, SM, SH, SD = p['engraving'], p['silver-light'], p['silver-mid'], p['silver-shade'], '#625D61'
    HI = p['silver-high']
    GM, GL, GD, GY, BO = p['panel-mid'], p['panel-light'], p['panel-shadow'], p['panel-stroke'], p['bordeaux']
    # GOODS: a silver plate with a 4x4 bay of black pockets, each with a lit lower-left lip
    goods = plate(0, 0, 40, 56, 3, HI, '#F0F0F2', SD)
    for r_ in range(4):
        for c_ in range(4):
            x, y = 4 + c_ * 8.9, 3.4 + r_ * 12.9
            goods += rr(x, y, 7.2, 10.8, SH, 1.5) + rr(x + 0.4, y + 0.4, 6.8, 10.4, K, 1.3)
    goods += rr(2, 19.5, 36, 15.5, K, 1) + rr(2.5, 20, 35, 14.5, '#191A1C', 0.8)
    # MEMO: a bordeaux cover with a silver edge, silver rings, yellow page lines
    memo = plate(0, 0, 40, 56, 3, BO, SL, SD) + rr(3, 3, 34, 50, '#1A0A07', 1.8)
    for y in (6.5, 12.7, 18.9, 38.5, 44.7, 50.9):
        memo += ci(7, y, 2.2, SD) + ci(6.7, y - 0.3, 1.8, SL) + ci(6.4, y - 0.6, 0.7, HI)
    memo += rr(14, 6, 21, 1.4, GY) + rr(14, 9, 14, 1, GM)
    # SAVE: a silver case with a chamfer, a green label with the yellow stroke, the shutter black
    save = poly('0,3.5 3.5,0 34,0 40,6 40,36.5 36.5,40 3.5,40 0,36.5', SD) + poly('0,3.5 3.5,0 33.2,0 39,5.8 39,35.9 35.8,39 3.5,39 0,35.8', SL) + \
        poly('1.1,4 4,1.1 32.8,1.1 37.9,6.2 37.9,36.2 36.2,37.9 4,37.9 1.1,36.2', SM) + \
        rr(9, 1.1, 22, 13, SH) + rr(9, 1.1, 21.2, 12.2, HI) + rr(21, 3, 6, 9, K, 1) + \
        rr(6, 20, 28, 17.9, K, 1.2) + rr(7.5, 22, 25, 13, GM, 1) + rr(7.5, 22, 25, 4, GL, 1) + rr(9, 28.5, 22, 1, GY) + rr(9, 31, 15, 1, GY)
    return {'goods': (40, 56, goods), 'memo': (40, 56, memo), 'save': (40, 40, save)}


def label(text, size, fill, out, extra=()):
    run('-background', 'none', '-font', FONT, '-pointsize', size, '-fill', fill, *extra, 'label:' + text, out)


def tile(p):
    os.makedirs(OUT, exist_ok=True)
    X = 4
    for n, (w, h, body) in icons(p).items():
        open(os.path.join(OUT, n + '.svg'), 'w').write(svg(w, h, body))
        run('-background', 'none', '-density', 96 * X, os.path.join(OUT, n + '.svg'), os.path.join(OUT, n + '.png'))
    t = os.path.join(OUT, 'tile.png')
    # the plate: the DS dark blue, brighter in the middle (a vignette in three steps)
    c, e = p['ds-navy'], p['ds-navy-edge']
    run('-size', f'{256 * X}x{192 * X}', 'radial-gradient:' + c + '-' + e, t)
    for n, (x, y) in (('memo', (30, 40)), ('save', (108, 48)), ('goods', (186, 40))):
        run(t, os.path.join(OUT, n + '.png'), '-geometry', f'+{x * X}+{y * X}', '-composite', t)
    # the swatches, two rows
    sw = [('panel-light', 'свет панели'), ('panel-mid', 'панель'), ('panel-shadow', 'тень панели'), ('panel-stroke', 'жёлтый штрих'), ('base-green', 'плата'),
          ('silver-high', 'блик'), ('silver-light', 'серебро свет'), ('silver-mid', 'серебро'), ('silver-shade', 'серебро тень'), ('engraving', 'чёрный'),
          ('bordeaux', 'бордо'), ('gold', 'золото'), ('lamp-red', 'лампа'), ('felt', 'подложка PS2'), ('ds-navy', 'плита DS')]
    for i, (k, name) in enumerate(sw):
        x, y = 12 + (i % 8) * 126, 560 + (i // 8) * 100
        run(t, '-fill', p[k], '-draw', f'rectangle {x},{y} {x + 110},{y + 56}', '-fill', '#ffffff55', '-draw', f'rectangle {x},{y} {x + 110},{y}', t)
        label(f'{name}\\n{p[k]}', 15, '#dfe3ea', os.path.join(OUT, 'lab.png'))
        run(t, os.path.join(OUT, 'lab.png'), '-geometry', f'+{x}+{y + 58}', '-composite', t)
    os.remove(os.path.join(OUT, 'lab.png'))
    print('tile:', t)


def main():
    a = sys.argv[1:]
    p = measure() if '--tile' not in a or '--measure' in a else None
    if '--tile' in a or not a:
        if p is None:
            p = {l.split('\t')[0]: l.split('\t')[1] for l in open(os.path.join(OUT, 'palette.txt'))}
        tile(p)


if __name__ == '__main__':
    main()
