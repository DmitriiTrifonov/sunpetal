#!/usr/bin/env python3
"""Optional step: make the big PS2 textures in a `.spdata` bigger (and a little cleaner).

    upscale.py IN.spdata OUT.spdata [--factor 2] [--max-side 512] [--only-larger-than 64]
                                    [--method lanczos-sharp] [--limit N] [--dry-run]

The renderer samples a replacement by the DS texture's own coordinates, so a picture of any size
fits under the same key; the file format does not change (`spdata.write`). Only ImageMagick
(`magick`) and `ffmpeg` that are already on the machine are used, called with argument lists, no
shell. Nothing is downloaded.

Colour and alpha are processed apart. The colour of fully transparent pixels is first filled from
their opaque neighbours (otherwise the filter pulls the hidden black or magenta of the cut-out into
the edge: dark halos), then the colour is enlarged by the chosen method and the alpha by Lanczos
with a mild contrast, and the two are joined again.

Methods:
    lanczos         plain Lanczos
    lanczos-sharp   Lanczos + light unsharp mask (default)
    catrom          Catmull-Rom
    mitchell        Mitchell-Netravali (softer)
    adaptive        ImageMagick -adaptive-resize
    lanczos-median  enlarge 2x more, 3x3 median, scale back (flattens JPEG-like noise, keeps edges)
    ffmpeg-lanczos  ffmpeg scale=flags=lanczos
    xbr             ffmpeg xbr (pixel-art; 2, 3, 4 only)
    hqx             ffmpeg hqx (pixel-art; 2, 3, 4 only)
    upscayl         neural (an installed Upscayl: model upscayl-standard-4x; colour only, the alpha stays Lanczos)
    upscayl-remacri the same with the remacri model

`--dry-run` estimates the sizes: old PNG size x area ratio x a measured constant (0.65 for palette
results, 0.85 for true colour). A guide, not a promise.
"""
import argparse
import collections
import hashlib
import os
import shutil
import struct
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import spdata

PNG_OPTS = ['-strip', '-define', 'png:compression-level=9']
SHARP = ['-unsharp', '0x0.7+0.7+0.02']
# PNG growth per unit of area ratio, measured on 7 textures: a 256-colour result (palette PNG, what
# --quantize gives to pictures that had 256 colours or fewer) 0.65, a true-colour one 0.85
SIZE_PER_AREA = {True: 0.65, False: 0.85}
MAX_COLORS = 256


def run(args):
    r = subprocess.run(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if r.returncode != 0:
        raise RuntimeError(f'{args[0]} failed ({r.returncode}): {r.stderr.decode(errors="replace")[:300]}')
    return r.stdout


def magick():
    exe = shutil.which('magick')
    if not exe:
        raise RuntimeError('ImageMagick (`magick`) not found')
    return exe


def png_size(png):
    return struct.unpack('>II', png[16:24])


def colors(path, mg):
    return int(run([mg, path, '-format', '%k', 'info:']))


def has_alpha(path, mg):
    out = run([mg, path, '-alpha', 'extract', '-format', '%[min]', 'info:'])
    return float(out) < 65535.0 * 0.999


# ---- colour methods: each gets (mg, src_rgb, dst_rgb, w, h, f) -------------------------------

def _resize(filt, post=()):
    def go(mg, src, dst, w, h, f):
        run([mg, src, '-filter', filt, '-resize', f'{w * f}x{h * f}!', *post, dst])
    return go


def _adaptive(mg, src, dst, w, h, f):
    run([mg, src, '-adaptive-resize', f'{w * f}x{h * f}!', dst])


def _median(mg, src, dst, w, h, f):
    run([mg, src, '-filter', 'Lanczos', '-resize', f'{w * f * 2}x{h * f * 2}!',
         '-statistic', 'Median', '3x3', '-filter', 'Lanczos', '-resize', f'{w * f}x{h * f}!', dst])


def _ffmpeg(flt):
    def go(mg, src, dst, w, h, f):
        ff = shutil.which('ffmpeg')
        if not ff:
            raise RuntimeError('ffmpeg not found')
        if flt in ('xbr', 'hqx') and f not in (2, 3, 4):
            raise RuntimeError(f'{flt} only does factors 2, 3, 4')
        vf = {'lanczos': f'scale={w * f}:{h * f}:flags=lanczos+accurate_rnd+full_chroma_int',
              'xbr': f'xbr=n={f}', 'hqx': f'hqx=n={f}'}[flt]
        run([ff, '-v', 'error', '-y', '-i', src, '-vf', vf, '-frames:v', '1', dst])
    return go


def _upscayl(model):
    """The neural upscaler of an installed Upscayl (its command line, called with an argument list; the
    pictures stay on the machine). `SUNPETAL_UPSCAYL` names the folder holding `bin/upscayl-bin` and
    `models/` (default the app's resources). The models are 4x; a smaller factor is the scaled result."""
    def go(mg, src, dst, w, h, f):
        root = os.environ.get('SUNPETAL_UPSCAYL', '/Applications/Upscayl.app/Contents/Resources')
        exe = os.path.join(root, 'bin', 'upscayl-bin')
        if not os.path.exists(exe):
            raise RuntimeError(f'upscayl-bin not found at {exe} (install Upscayl or set SUNPETAL_UPSCAYL)')
        run([exe, '-i', src, '-o', dst, '-m', os.path.join(root, 'models'), '-n', model, '-z', '4', '-s', str(f)])
    return go


METHODS = collections.OrderedDict([
    ('lanczos', _resize('Lanczos')),
    ('lanczos-sharp', _resize('Lanczos', SHARP)),
    ('catrom', _resize('Catrom')),
    ('mitchell', _resize('Mitchell')),
    ('adaptive', _adaptive),
    ('lanczos-median', _median),
    ('ffmpeg-lanczos', _ffmpeg('lanczos')),
    ('xbr', _ffmpeg('xbr')),
    ('hqx', _ffmpeg('hqx')),
    ('upscayl', _upscayl('upscayl-standard-4x')),
    ('upscayl-remacri', _upscayl('remacri-4x')),
])


def bleed(mg, src, dst, tmp):
    """Fill the colour of transparent pixels from the opaque ones (two radii), drop the alpha."""
    j = lambda n: os.path.join(tmp, n)
    run([mg, src, '-background', 'black', '-alpha', 'remove', '-alpha', 'off', j('p.png')])   # premultiplied
    run([mg, src, '-alpha', 'extract', j('a.png')])
    for i, sg in enumerate((2, 8)):
        run([mg, j('p.png'), '-blur', f'0x{sg}', j(f'pb{i}.png')])
        run([mg, j('a.png'), '-blur', f'0x{sg}', j(f'ab{i}.png')])
        # blurred colour / blurred alpha = average colour of the opaque neighbourhood
        run([mg, j(f'pb{i}.png'), j(f'ab{i}.png'), '-compose', 'Divide_Src', '-composite', j(f'd{i}.png')])
    # where the near fill has support (blurred alpha > 3%) use it, else the wide one
    run([mg, j('ab0.png'), '-threshold', '3%', j('near.png')])
    run([mg, j('d1.png'), j('d0.png'), j('near.png'), '-compose', 'Over', '-composite', j('base.png')])
    run([mg, j('a.png'), '-threshold', '0', j('keep.png')])      # alpha > 0: the pixel keeps its colour
    run([mg, src, '-background', 'black', '-alpha', 'off', j('plain.png')])
    run([mg, j('base.png'), j('plain.png'), j('keep.png'), '-compose', 'Over', '-composite', dst])


def enlarge(png, method, f, mg, tmp, quantize='auto'):
    """-> PNG bytes enlarged f times (colour by `method`, alpha by Lanczos). With quantize 'auto' a
    picture that had at most 256 colours (the PS2 palette textures) comes back as a 256-colour
    palette PNG, a third of the size and visually the same; 'always' / 'never' force it."""
    w, h = png_size(png)
    src = os.path.join(tmp, 'src.png')
    with open(src, 'wb') as fh:
        fh.write(png)
    out = os.path.join(tmp, 'out.png')
    pal = quantize == 'always' or (quantize == 'auto' and colors(src, mg) <= MAX_COLORS)
    tail = ['-dither', 'None', '-colors', str(MAX_COLORS), 'PNG8:' + out] if pal else [out]
    if not has_alpha(src, mg):
        col, col_up = os.path.join(tmp, 'c0.png'), os.path.join(tmp, 'c1.png')
        run([mg, src, '-alpha', 'off', col])
        METHODS[method](mg, col, col_up, w, h, f)
        run([mg, col_up, '-alpha', 'off', *PNG_OPTS, *tail])
    else:
        col, col_up = os.path.join(tmp, 'c0.png'), os.path.join(tmp, 'c1.png')
        a_up = os.path.join(tmp, 'a1.png')
        bleed(mg, src, col, tmp)
        METHODS[method](mg, col, col_up, w, h, f)
        run([mg, src, '-alpha', 'extract', '-filter', 'Lanczos', '-resize', f'{w * f}x{h * f}!',
             '-level', '12%,88%', a_up])
        run([mg, col_up, '-alpha', 'off', a_up, '-compose', 'CopyOpacity', '-composite', *PNG_OPTS, *tail])
    with open(out, 'rb') as fh:
        return fh.read()


def plan_factor(w, h, factor, max_side, only_larger):
    """The factor this picture gets, 1 = leave it."""
    long = max(w, h)
    if long <= only_larger:
        return 1
    f = factor
    while f > 1 and long * f > max_side:
        f -= 1
    return f


def main(argv=None):
    ap = argparse.ArgumentParser(description='enlarge the textures of a .spdata', formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument('input')
    ap.add_argument('output', nargs='?')
    ap.add_argument('--factor', type=int, default=2, help='integer factor, 2..4 (default 2)')
    ap.add_argument('--max-side', type=int, default=512, help='no result longer than this (default 512)')
    ap.add_argument('--only-larger-than', type=int, default=64, metavar='N',
                    help='only pictures whose longer side is more than N pixels (default 64)')
    ap.add_argument('--method', choices=list(METHODS), default='lanczos-sharp')
    ap.add_argument('--quantize', choices=('auto', 'always', 'never'), default='auto',
                    help='256-colour palette PNG: auto = only where the source had <=256 colours (default)')
    ap.add_argument('--limit', type=int, default=0, help='only the first N pictures that qualify (for tests)')
    ap.add_argument('--dry-run', action='store_true', help='print the plan and the size estimate, write nothing')
    a = ap.parse_args(argv)
    if not 2 <= a.factor <= 4:
        ap.error('--factor must be 2, 3 or 4')
    if not a.dry_run and not a.output:
        ap.error('OUTPUT is required without --dry-run')

    sections, textures = spdata.read(a.input)
    for tag in sections:
        if tag != 'TEXS':
            print(f'warning: section {tag} is not copied (spdata.write only writes TEXS)', file=sys.stderr)
    pics = {}        # png hash -> (png, [keys])
    for key, entry in sorted(textures.items()):
        png = spdata.png_of(a.input, entry)
        pics.setdefault(hashlib.sha1(png).digest(), (png, []))[1].append(key)

    plan = []
    for hsh, (png, keys) in pics.items():
        w, h = png_size(png)
        plan.append((hsh, png, keys, w, h, plan_factor(w, h, a.factor, a.max_side, a.only_larger_than)))
    todo = [p for p in plan if p[5] > 1]
    if a.limit:
        todo = todo[:a.limit]
    chosen = {p[0] for p in todo}

    mg0 = magick()
    pal_of = {}
    with tempfile.TemporaryDirectory(prefix='sp-upscale-') as t0:
        for hsh, png, keys, w, h, f in plan:
            if hsh in chosen:
                src = os.path.join(t0, 'c.png')
                with open(src, 'wb') as fh:
                    fh.write(png)
                pal_of[hsh] = a.quantize == 'always' or (a.quantize == 'auto' and colors(src, mg0) <= MAX_COLORS)

    groups = collections.OrderedDict()      # (w,h,f) -> [count, old bytes, new bytes estimate]
    for hsh, png, keys, w, h, f in plan:
        if hsh not in chosen:
            f = 1
        g = groups.setdefault((w, h, f), [0, 0, 0])
        g[0] += 1
        g[1] += len(png)
        g[2] += int(len(png) * f * f * SIZE_PER_AREA[pal_of[hsh]]) if f > 1 else len(png)
    print(f'{len(textures)} keys, {len(pics)} distinct pictures; method {a.method}, factor {a.factor}, '
          f'max side {a.max_side}, only longer than {a.only_larger_than}')
    print(f'{"size":>9} {"->":>9} {"count":>6} {"now KB":>8} {"est. KB":>8}')
    for (w, h, f), (n, old, new) in sorted(groups.items(), key=lambda kv: -kv[1][1]):
        print(f'{w}x{h:<5} {"" if f == 1 else f"{w * f}x{h * f}":>9} {n:>6} {old // 1024:>8} {new // 1024:>8}')
    old = sum(g[1] for g in groups.values())
    new = sum(g[2] for g in groups.values())
    print(f'to enlarge: {len(todo)} pictures; PNG total {old / 1e6:.2f} MB -> about {new / 1e6:.2f} MB')
    if a.dry_run:
        return 0

    mg = magick()
    out_png = {}
    with tempfile.TemporaryDirectory(prefix='sp-upscale-') as tmp:
        for i, (hsh, png, keys, w, h, f) in enumerate(todo):
            try:
                out_png[hsh] = enlarge(png, a.method, f, mg, tmp, a.quantize)
            except RuntimeError as e:
                print(f'skip {w}x{h}: {e}', file=sys.stderr)
            if (i + 1) % 25 == 0 or i + 1 == len(todo):
                print(f'{i + 1}/{len(todo)}', file=sys.stderr)
    items = []
    for hsh, (png, keys) in pics.items():
        for key in keys:
            items.append((key, out_png.get(hsh, png)))
    spdata.write(a.output, items)
    print(f'wrote {a.output}: {len(items)} keys, {os.path.getsize(a.output) / 1e6:.2f} MB')
    return 0


if __name__ == '__main__':
    sys.exit(main())
