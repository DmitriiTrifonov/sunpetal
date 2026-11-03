#!/usr/bin/env python3
"""8е.7ж: the case menu's icons, the selection frame and the return arrow, cut for the 2D pack (2DPK).

The OBJ sprites of the case menu (engine B) are replaced by key: every sprite of an icon gets the piece of the
redrawn icon (at 4x) under its own rectangle, with the label (EN or RU) set over it. The frame's corners are cut
from the frame alone (the frame moves from one icon to another), the arrow from the arrow alone.

  menu_icons_pack.py --art DIR --fonts DIR --dumps DIR/en,DIR/ru --out PACK_DIR

--art     the folder with guide.svg, memo_big.png, goods_big.png, save_big.png, arrow_big.png (artifacts/2d-samples/redraw)
--fonts   build/fonts (FiraSansExtraCondensed-Medium.ttf)
--dumps   a folder with `index.csv` of `dump2d` on the menu frame, one per language (en, ru)
--bodies  optional: a folder with guide_big.png, memo_big.png, save_big.png, goods_big.png (`icons_epx_art.py`); they replace
          the redrawn bodies, the frame (guide.svg) and the arrow stay from --art
--out     the pack folder: obj1_WxH_DATA_PALETTE.png files are written there
"""
import argparse, csv, os, re, subprocess, tempfile

X = 4
# DS pixel positions of the redrawn art on the lower screen (the same as `panel.py`)
POS = {'guide': (28, 61), 'memo': (82, 64), 'save': (133, 76), 'goods': (184, 64), 'arrow': (220, 171), 'port': (141, 64)}
# icon sprites: OAM index ranges of the menu's items
ITEMS = {'guide': range(26, 32), 'memo': range(32, 38), 'save': range(38, 42), 'goods': range(42, 48)}
FRAME = {22: (28, 61), 23: (28, 91), 24: (28, 61), 25: (28, 91)}     # the left corner pieces cut for the (mirrored) right ones too
ARROW = (0, 1)
SETS = {
    'en': [('GUIDE\nBOOK', 51, 91, 8), ('MEMO', 102, 92, 8.5), ('SAVE', 153, 96, 8.5), ('GOODS', 204, 92, 8.5)],
    'ru': [('ГИД', 51, 92, 7), ('ЗАМЕТКИ', 102, 92, 5.6), ('ЗАПИСЬ', 153, 96, 6), ('ВЕЩИ', 204, 92, 7)],
}


# the puzzle's strip (B-13): MEMO, PORT, GOODS; the sprites there are numbered otherwise, so every sprite but the frame's
# and the arrow's is cut by its place from the strip's own canvas (PORT would lie over SAVE on the menu's)
PUZZLE = {
    'en': [('MEMO', 102, 92, 8.5), ('PORT', 153, 92, 8), ('GOODS', 204, 92, 8.5)],
    'ru': [('ЗАМЕТКИ', 102, 92, 5.6), ('ПОРТ', 153, 92, 6.5), ('ВЕЩИ', 204, 92, 7)],
}

# the menu before the GUIDE is found: MEMO, SAVE, GOODS only (OAM 26-41), cut by place like the puzzle's strip
TRIO = {lang: [SETS[lang][1], SETS[lang][2], SETS[lang][3]] for lang in SETS}

# labels over a busy body get a black band (DS x range, past the memo's holes, across the goods' cells)
PLATE = {'MEMO': (83, 121), 'ЗАМЕТКИ': (83, 121), 'GOODS': (185, 223), 'ВЕЩИ': (185, 223)}


def run(*a):
    subprocess.run(['magick', *map(str, a)], check=True)


def label(font, text, cap, out, tmp):
    pt = int(cap * X / 0.64)
    run('-background', 'none', '-font', font, '-pointsize', pt, '-fill', 'black', '-stroke', 'black', '-strokewidth', X * 0.9, 'label:' + text,
        '-bordercolor', 'none', '-border', 12, '+repage', tmp + '/l_back.png')
    run('-background', 'none', '-font', font, '-pointsize', pt, '-fill', 'white', '-stroke', 'none', 'label:' + text,
        '-bordercolor', 'none', '-border', 12, '+repage', tmp + '/l_front.png')
    run(tmp + '/l_back.png', tmp + '/l_front.png', '-gravity', 'center', '-composite', out)


def put(base, lab, text, cx, cy, tmp):
    """The label (on its band when it has one) onto `base`, centred at DS point (cx, cy)."""
    w, h = size(lab)
    if text in PLATE:
        x0, x1 = PLATE[text]
        bh = h - 16
        run('-size', f'{(x1 - x0) * X}x{bh}', 'xc:black', tmp + '/band.png')
        run(base, tmp + '/band.png', '-geometry', f'+{x0 * X}+{int(cy * X - bh / 2)}', '-composite', base)
    run(base, lab, '-geometry', f'+{int(cx * X - w / 2)}+{int(cy * X - h / 2)}', '-composite', base)


def size(path):
    return tuple(int(v) for v in subprocess.check_output(['magick', 'identify', '-format', '%w %h', path]).split())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--art', required=True)
    ap.add_argument('--fonts', required=True)
    ap.add_argument('--dumps', default='', help='en=DIR,ru=DIR (the case menu)')
    ap.add_argument('--puzzle-dumps', default='', help='en=DIR,ru=DIR: `dump2d` on the strip with PORT (a puzzle)')
    ap.add_argument('--trio-dumps', default='', help='en=DIR,ru=DIR: `dump2d` on the menu without GUIDE (MEMO, SAVE, GOODS)')
    ap.add_argument('--bodies', default='')
    ap.add_argument('--out', required=True)
    a = ap.parse_args()
    font = a.fonts + '/firasansextracondensed/FiraSansExtraCondensed-Medium.ttf'
    dumps = dict(p.split('=') for p in a.dumps.split(',') if p)
    pdumps = dict(p.split('=') for p in a.puzzle_dumps.split(',') if p)
    tdumps = dict(p.split('=') for p in a.trio_dumps.split(',') if p)
    os.makedirs(a.out, exist_ok=True)
    tmp = tempfile.mkdtemp(prefix='menu_icons_')
    svg = open(a.art + '/guide.svg').read()
    frame_only = re.sub(r'<rect[^>]*/>', '', svg)                     # only the four orange corners
    body_only = re.sub(r'<polygon[^>]*/>', '', svg)                   # only the card
    open(tmp + '/guide_body.svg', 'w').write(body_only)
    open(tmp + '/guide_frame.svg', 'w').write(frame_only)
    for n in ('guide_body', 'guide_frame'):
        run('-background', 'none', '-density', 96 * X, tmp + f'/{n}.svg', tmp + f'/{n}.png')
    art = {'guide': tmp + '/guide_body.png', 'memo': a.art + '/memo_big.png', 'save': a.art + '/save_big.png', 'goods': a.art + '/goods_big.png'}
    if a.bodies:
        art = {k: f'{a.bodies}/{k}_big.png' for k in art}
    W, H = 256 * X, 192 * X

    def canvas(layers, out):
        cmd = ['-size', f'{W}x{H}', 'xc:none']
        for img, (x, y) in layers:
            cmd += [img, '-geometry', f'+{x * X}+{y * X}', '-composite']
        run(*cmd, out)

    canvas([(tmp + '/guide_frame.png', POS['guide'])], tmp + '/frame.png')
    canvas([(a.art + '/arrow_big.png', POS['arrow'])], tmp + '/arrow.png')
    done = {}
    for lang, labels in SETS.items():
        if lang not in dumps:
            continue
        bodies = tmp + f'/bodies_{lang}.png'
        canvas([(art[k], POS[k]) for k in ('guide', 'memo', 'save', 'goods')], bodies)
        for i, (t, cx, cy, cap) in enumerate(labels):
            lab = tmp + f'/lab_{lang}_{i}.png'
            label(font, t, cap, lab, tmp)
            put(bodies, lab, t, cx, cy, tmp)
        with open(dumps[lang] + '/index.csv', newline='') as f:
            rows = [r for r in csv.DictReader(f) if r['kind'] == 'obj' and r['engine'] == '1']
        for r in rows:
            idx, x, y, w, h = (int(r[k]) for k in ('layer_or_oam', 'x', 'y', 'w', 'h'))
            src = None
            if any(idx in rg for rg in ITEMS.values()):
                src = bodies
            elif idx in FRAME:
                src, (x, y) = tmp + '/frame.png', FRAME[idx]
            elif idx in ARROW:
                src = tmp + '/arrow.png'
            if src is None:
                continue
            name = r['file']
            piece = a.out + '/' + name
            run(src, '-crop', f'{w * X}x{h * X}+{x * X}+{y * X}', '+repage', piece)
            # the same key in both languages must be the same picture (a piece without the label)
            if name in done and done[name] != (lang, src, x, y):
                if lang != done[name][0] and src == bodies:
                    print('note: key', name, 'shared by', done[name][0], 'and', lang)
            done[name] = (lang, src, x, y)
    jobs = [('puzzle', lang, labels, pdumps, ('memo', 'port', 'goods')) for lang, labels in PUZZLE.items()]
    jobs += [('trio', lang, labels, tdumps, ('memo', 'save', 'goods')) for lang, labels in TRIO.items()]
    for kind, lang, labels, pdumps, items in jobs:
        if lang not in pdumps:
            continue
        strip = tmp + f'/strip_{kind}_{lang}.png'
        canvas([(a.art + '/port_big.png' if k == 'port' else art[k], POS[k]) for k in items], strip)
        for i, (t, cx, cy, cap) in enumerate(labels):
            lab = tmp + f'/plab_{kind}_{lang}_{i}.png'
            label(font, t, cap, lab, tmp)
            put(strip, lab, t, cx, cy, tmp)
        with open(pdumps[lang] + '/index.csv', newline='') as f:
            rows = [r for r in csv.DictReader(f) if r['kind'] == 'obj' and r['engine'] == '1']
        for r in rows:
            idx, x, y, w, h = (int(r[k]) for k in ('layer_or_oam', 'x', 'y', 'w', 'h'))
            if idx in FRAME:
                src, (x, y) = tmp + '/frame.png', FRAME[idx]
            elif idx in ARROW:
                src = tmp + '/arrow.png'
            else:
                src = strip
            run(src, '-crop', f'{w * X}x{h * X}+{x * X}+{y * X}', '+repage', a.out + '/' + r['file'])
            done[r['file']] = (lang, src, x, y)
        print(kind, 'strip', lang, 'pieces:', len(rows))
    print('pieces:', len(done), '->', a.out)


if __name__ == '__main__':
    main()
