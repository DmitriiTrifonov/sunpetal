#!/usr/bin/env python3
"""K-123: the start-up logos (8е.1 № 18) enlarged 4x for the 2D pack.

    python3 tools/ps2/logo_pack.py --dump DIR --out build/2d-pack-case [--method upscayl]

DIR is a `--dump-2d` folder of a run of the first 800 frames (sunpetal-headless ROM --vblanks 820
--shots 280,400,600,750 --dump-2d DIR): the full-screen BG layers (`bgNN_<key>.png`, 256x192) are the logos.
Each is enlarged by `tools/ps2/upscale.py` (default `upscayl`, the crisp text of flat logos) and written under the
same name, which is the key the engine looks a layer up by. The pictures are the game's, so they stay out of git.
"""
import argparse, os, sys, tempfile
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import upscale


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--dump', required=True)
    ap.add_argument('--out', required=True)
    ap.add_argument('--method', default='upscayl')
    ap.add_argument('--factor', type=int, default=4)
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    for f in sorted(os.listdir(a.dump)):
        if not (f.startswith('bg') and f.endswith('.png')):
            continue
        png = open(os.path.join(a.dump, f), 'rb').read()
        with tempfile.TemporaryDirectory() as tmp:
            open(os.path.join(a.out, f), 'wb').write(upscale.enlarge(png, a.method, a.factor, 'magick', tmp, 'never'))
        print(f)


if __name__ == '__main__':
    main()
