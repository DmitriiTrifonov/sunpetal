#!/usr/bin/env python3
"""8e.7: enlarge the PS2 case pictures (the dump of `case.py`) by several methods and lay out sheets to compare.

    case_up.py [--png build/ps2-case/png] [--out build/ps2-case/up] [--ds artifacts/menu8g/device.png]
               [--factors 2 3 4] [--methods lanczos-sharp catrom adaptive lanczos-median upscayl]

Only ImageMagick and what `upscale.py` already calls (an installed Upscayl is used when it is there,
nothing is downloaded or installed; methods that cannot run are skipped with a note). Output:
`OUT/<picture>/<method>_x<f>.png` and the comparison sheets in `OUT/sheets/` (nothing goes to git).
"""
import argparse
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import upscale

ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))
FONT = os.path.join(ROOT, 'build/fonts/firasansextracondensed/FiraSansExtraCondensed-Medium.ttf')
BG = '#3a4660'

# (short name, file prefix in the dump) of the pictures that matter for the case
PICTURES = [
    ('case_book', '016_004'), ('case_empty', '016_006'),
    ('schema', '016_000'), ('schema_panel', '016_002'),
    ('digits', '016_094'), ('digits_small', '016_092'),
    ('lamp', '016_028'), ('push_a', '016_030'), ('push_b', '016_032'),
    ('push_digit0', '015_000'), ('strip', '016_090'),
]
# crops of the case render (x, y, w, h): the disc with the digit window and the button strip
CROPS = {'disc': (160, 70, 160, 140), 'strip_render': (112, 76, 44, 116)}


def mg(*a):
    upscale.run(['magick', *map(str, a)])


def find(png_dir, prefix):
    for f in sorted(os.listdir(png_dir)):
        if f.startswith(prefix + '_'):
            return os.path.join(png_dir, f)
    raise FileNotFoundError(prefix)


def enlarge_file(src, dst, method, f):
    with open(src, 'rb') as fh:
        png = fh.read()
    with tempfile.TemporaryDirectory() as tmp:
        data = upscale.enlarge(png, method, f, upscale.magick(), tmp, quantize='never')
    with open(dst, 'wb') as fh:
        fh.write(data)


def nearest(src, dst, f):
    mg(src, '-filter', 'point', '-resize', f'{f * 100}%', dst)


def sheet(cells, out, tile):
    """cells: [(label, path)], laid out in a grid on the panel colour."""
    args = ['montage', '-font', FONT, '-pointsize', '16', '-fill', 'white', '-background', BG, '-tile', tile, '-geometry', '+8+8']
    for label, path in cells:
        args += ['-label', label, path]
    mg(*args, out)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument('--png', default=os.path.join(ROOT, 'build/ps2-case/png'))
    ap.add_argument('--out', default=os.path.join(ROOT, 'build/ps2-case/up'))
    ap.add_argument('--ds', default=os.path.join(ROOT, 'artifacts/menu8g/device.png'))
    ap.add_argument('--factors', type=int, nargs='+', default=[2, 3, 4])
    ap.add_argument('--methods', nargs='+', default=['lanczos-sharp', 'catrom', 'adaptive', 'lanczos-median', 'upscayl'])
    a = ap.parse_args(argv)
    sheets = os.path.join(a.out, 'sheets')
    os.makedirs(sheets, exist_ok=True)

    srcs = {n: find(a.png, p) for n, p in PICTURES}
    case = srcs['case_empty']
    for n, (x, y, w, h) in CROPS.items():
        d = os.path.join(a.out, '_src')
        os.makedirs(d, exist_ok=True)
        srcs[n] = os.path.join(d, n + '.png')
        mg(case, '-crop', f'{w}x{h}+{x}+{y}', '+repage', srcs[n])

    skipped = {}
    for name, src in srcs.items():
        d = os.path.join(a.out, name)
        os.makedirs(d, exist_ok=True)
        for m in a.methods:
            for f in a.factors:
                dst = os.path.join(d, f'{m}_x{f}.png')
                if os.path.exists(dst):
                    continue
                try:
                    enlarge_file(src, dst, m, f)
                except (RuntimeError, OSError) as e:
                    skipped[m] = str(e)[:120]
        # comparison sheet at x3: the original (pixel-doubled to the same size) and every method
        f = 3 if 3 in a.factors else a.factors[0]
        ref = os.path.join(d, f'nearest_x{f}.png')
        nearest(src, ref, f)
        cells = [('original (nearest)', ref)] + [(m, os.path.join(d, f'{m}_x{f}.png')) for m in a.methods
                                                 if os.path.exists(os.path.join(d, f'{m}_x{f}.png'))]
        sheet(cells, os.path.join(sheets, f'{name}_x{f}.png'), f'{min(len(cells), 3)}x')

    # the case x3 beside the DS picture (the DS upper screen, x3 by pixels)
    best = a.methods[0]
    ds_top = os.path.join(a.out, '_src', 'ds_top.png')
    mg(a.ds, '-crop', '256x192+0+0', '+repage', ds_top)
    ds3 = os.path.join(a.out, '_src', 'ds_top_x3.png')
    nearest(ds_top, ds3, 3)
    up = os.path.join(a.out, 'case_empty', f'{best}_x3.png')
    out = os.path.join(sheets, 'case_x3_vs_ds.png')
    sheet([('DS (x3, pixels)', ds3), (f'PS2 render x3 ({best})', up)], out, '2x')
    for m, why in skipped.items():
        print(f'skipped {m}: {why}', file=sys.stderr)
    print('sheets:', sheets)


if __name__ == '__main__':
    main()
