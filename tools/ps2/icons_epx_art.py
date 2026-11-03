#!/usr/bin/env python3
"""K-116 (8е.4в): bodies of the case menu's icons from the DS originals, labels removed, enlarged x4 by EPX.

The originals (`guide/memo/save/goods_original.png`, made by `tools/samples-2d.py` from `dump2d`) carry their
label baked in. The labels are wiped by the icons' own rules (borders, the notebook's holes, the grid's cells) and
the result goes through EPX twice (x4). The labels are set over the bodies later by `menu_icons_pack.py --bodies`.

    icons_epx_art.py SAMPLES_DIR OUT_DIR

Writes OUT_DIR/{guide,memo,save,goods}_big.png (nothing of the game goes into the repository).
"""
import importlib.util, os, sys

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location('samples2d', os.path.join(HERE, '..', 'samples-2d.py'))
s2 = importlib.util.module_from_spec(spec)
_argv, sys.argv = sys.argv, ['samples-2d.py']
try:
    spec.loader.exec_module(s2)
except SystemExit:
    pass
sys.argv = _argv

WHITE = (251, 251, 251, 255)
BLACK = (0, 0, 0, 255)


def band(im, y0, y1, x_white_l, x_black0, x_black1, x_white_r):
    """Rows y0..y1: the border columns white, the inside black."""
    for y in range(y0, y1 + 1):
        for x in x_white_l:
            im[y][x] = WHITE
        for x in range(x_black0, x_black1 + 1):
            im[y][x] = BLACK
        for x in x_white_r:
            im[y][x] = WHITE


def hole(im, top):
    """A hole of the notebook's binding at x 2..5 from row `top` (the shape of the ones that are not covered)."""
    for dy, xs in enumerate(((3, 4), (2, 3, 4, 5), (2, 3, 4, 5), (3, 4))):
        for x in xs:
            im[top + dy][x] = WHITE


def clean_memo(im):
    band(im, 22, 33, (0,), 1, 38, (39,))
    hole(im, 21)
    hole(im, 28)


def clean_save(im):
    for x in range(14, 17):
        im[14][x] = WHITE
    band(im, 15, 25, (0,), 1, 38, (39,))
    for x in (36, 37):
        im[37][x] = BLACK      # a transparent gap in the original's lower corner


def clean_goods(im):
    for y in range(22, 27):
        im[y] = list(im[21])
    for y in (27, 28):
        im[y] = list(im[14])
    for y in range(29, 33):
        im[y] = list(im[33])


def clean_guide(im):
    """The card is a white-bordered black rectangle; the frame's orange brackets (its own piece) lie over its corners
    and the labels over its middle: it is rebuilt by rule (x 3..42, y 3..54, the border 3 wide on the sides and
    top, 5 at the bottom) and nothing outside it stays."""
    for y, row in enumerate(im):
        for x in range(len(row)):
            if not (3 <= x <= 42 and 3 <= y <= 54):
                row[x] = (0, 0, 0, 0)
            elif x <= 5 or x >= 40 or y <= 5 or y >= 50:
                row[x] = WHITE
            else:
                row[x] = BLACK


CLEAN = {'memo': clean_memo, 'save': clean_save, 'goods': clean_goods, 'guide': clean_guide}


def main():
    src, out = sys.argv[1:3]
    os.makedirs(out, exist_ok=True)
    for name, fn in CLEAN.items():
        im = [list(r) for r in s2.read_png(os.path.join(src, f'{name}_original.png'))]
        fn(im)
        s2.write_png(os.path.join(out, f'{name}_clean.png'), im)
        s2.write_png(os.path.join(out, f'{name}_big.png'), s2.epx(s2.epx(im)))
        print(name, len(im[0]), 'x', len(im), '->', len(im[0]) * 4, 'x', len(im) * 4)


main()
