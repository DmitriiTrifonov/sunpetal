#!/usr/bin/env python3
"""The case-menu's selection frame: redraw its six 16x32 corner states at 4x.

The DS sprite is a 16x32 tile with an orange corner (an 8x8 L in 1x) drawn at a
per-state offset; the frame "breathes" by moving the corners diagonally. For the
4x art swap we need, for every sprite, a 64x128 PNG holding the redrawn corner
(from guide.svg) at the same anchor as the original 1x sprite:

  top    corners: anchor = the corner's top-left    at (bbox.x*4, bbox.y*4)
  bottom corners: anchor = the corner's bottom-left at (bbox.x*4, (bbox.y+bbox.h)*4)

The bbox is the bounding box of the sprite's opaque pixels (alpha > 0), in 1x.
The redrawn corner keeps its svg shape/size (rendered at 4x), it is not stretched
to the bbox.

  frame_states_pack.py --dumps DIR --art DIR --out DIR
"""
import argparse, os, re, shutil, subprocess, tempfile

X = 4
CANVAS_W, CANVAS_H = 64, 128          # 16x32 at 4x
SVG_HEAD = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 46 62" width="46" height="62">'


def run(*a):
    subprocess.run(['magick', *map(str, a)], check=True)


def out(*a):
    return subprocess.check_output(['magick', *map(str, a)])


def image_size(path):
    w, h = out('identify', '-format', '%w %h', path).decode().split()
    return int(w), int(h)


def bbox_alpha(path):
    """(x, y, w, h) of the pixels with alpha > 0, in the image's own units."""
    w, h = image_size(path)
    raw = out(path, '-alpha', 'extract', '-depth', '8', 'gray:-')
    minx, miny, maxx, maxy = w, h, -1, -1
    for y in range(h):
        row = raw[y * w:(y + 1) * w]
        for x in range(w):
            if row[x] > 0:
                if x < minx:
                    minx = x
                if x > maxx:
                    maxx = x
                if y < miny:
                    miny = y
                if y > maxy:
                    maxy = y
    if maxx < 0:
        return (0, 0, 0, 0)
    return (minx, miny, maxx - minx + 1, maxy - miny + 1)


def corners(svg_text):
    """The four orange corner polygons; return (top_left, bottom_left)."""
    polys = re.findall(r'<polygon\b[^>]*?/>', svg_text)
    return polys[0], polys[2]


def render_corner(polygon, name, tmp):
    svg_path = tmp + '/' + name + '.svg'
    open(svg_path, 'w').write(SVG_HEAD + polygon + '</svg>')
    png = tmp + '/' + name + '.png'
    run('-background', 'none', '-density', 96 * X, svg_path, png)
    return png


def is_bottom(bbox, sprite_h):
    return bbox[1] + bbox[3] > sprite_h // 2


def anchor(bbox, bottom):
    """The redrawn corner's anchor point at 4x, from the 1x sprite bbox."""
    x, y, w, h = bbox
    return (x * X, (y + h) * X if bottom else y * X)


def compose(corner_png, bbox, bottom, out_path):
    """Place the redrawn corner (its own units) onto the 64x128 canvas.

    The corner's geometric anchor point is (ax, ay): its top-left for the upper
    states, its bottom-left for the lower ones. The corner's tip pixel is the
    crop's top-left pixel (upper) or bottom-left pixel (lower), so the crop is
    offset by ay (upper) or ay - ch (lower).
    """
    cx, cy, cw, ch = bbox_alpha(corner_png)
    ax, ay = anchor(bbox, bottom)
    oy = ay - ch if bottom else ay
    run('-size', f'{CANVAS_W}x{CANVAS_H}', 'xc:none',
        '(', corner_png, '-crop', f'{cw}x{ch}+{cx}+{cy}', '+repage', ')',
        '-geometry', f'+{ax}+{oy}', '-composite', out_path)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--dumps', required=True)
    ap.add_argument('--art', required=True)
    ap.add_argument('--out', required=True)
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    svg = open(a.art + '/guide.svg').read()
    tl, bl = corners(svg)
    tmp = tempfile.mkdtemp(prefix='.frame_states_', dir=a.out)
    try:
        tl_png = render_corner(tl, 'tl', tmp)
        bl_png = render_corner(bl, 'bl', tmp)
        names = sorted(n for n in os.listdir(a.dumps) if n.endswith('.png'))
        rows = []
        for name in names:
            src = a.dumps + '/' + name
            w, h = image_size(src)
            bb = bbox_alpha(src)
            bottom = is_bottom(bb, h)
            compose(bl_png if bottom else tl_png, bb, bottom, a.out + '/' + name)
            ax, ay = anchor(bb, bottom)
            rows.append((name.split('_')[2], bb, (ax, ay)))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    print('key'.ljust(17), 'bbox (1x)'.ljust(12), 'anchor (4x)')
    for key, bb, (ax, ay) in rows:
        print(key.ljust(17), f'{bb[2]}x{bb[3]}+{bb[0]}+{bb[1]}'.ljust(12), f'{ax},{ay}')
    print('wrote', len(rows), 'sprites ->', a.out)


if __name__ == '__main__':
    main()
