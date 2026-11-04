#!/usr/bin/env python3
"""K-135: the plaque «управление набором» (the dial's help sign over the device) as pictures of the 2D pack.

    python3 tools/ps2/dial_label_pack.py --out build/2d-pack-case [--lang ru] [--fonts build/fonts]

The game draws the sign with 10 OBJ sprites at (44,72) .. (212,120) of the screen showing the device: a black plaque with a white
border and the text in pixel letters. Here the plaque is drawn at 4x (672x192) in a few fonts and cut to the sprites' rectangles;
each variant goes to the group `labels` (`obj0_<w>x<h>_<key>@labels.<variant>.png`), the game's own look is the variant `original`
(no picture). The keys come from a frame of the game (F10 with «Debug → write new 2D pictures» on), per language.
Only ImageMagick (`magick`) and the python standard library are used.
"""
import argparse, os, subprocess

X = 4
ORIGIN = (44, 72)
SIZE = (168, 48)                 # DS px
BORDER = 4                       # the white border, DS px
TEXT_BOX = (150 * X, 11 * X)     # the text is fitted into this (px at 4x); its centre is 22.5 DS px down from the plaque's top
TEXT_CY = int(22.5 * X)

# pieces: (x, y, w, h, key), screen coordinates (DS px); the four bottom strips in the middle are one picture
PIECES = {
    'ru': [(44, 72, 64, 32, '495d1c44b6227558_a0be8bf7745dbaf7'), (108, 72, 64, 32, '113e4a94c65ae0e3_a0be8bf7745dbaf7'),
           (172, 72, 32, 32, 'be58ecf70c203cc2_a0be8bf7745dbaf7'), (204, 72, 8, 32, 'd8adb8e9fb84ef25_da325e14afdc6bb3'),
           (44, 104, 32, 16, '0ba82a2e223d3f25_da325e14afdc6bb3'), (76, 104, 32, 16, '199df70e70535325_da325e14afdc6bb3'),
           (108, 104, 32, 16, '199df70e70535325_da325e14afdc6bb3'), (140, 104, 32, 16, '199df70e70535325_da325e14afdc6bb3'),
           (172, 104, 32, 16, '199df70e70535325_da325e14afdc6bb3'), (204, 104, 8, 16, '986a0b8080305b25_da325e14afdc6bb3')],
}
TEXTS = {'ru': 'УПРАВЛЕНИЕ НАБОРОМ'}
FONTS = {'fira': 'firasansextracondensed/FiraSansExtraCondensed-Medium.ttf',
         'philosopher': 'philosopher/Philosopher-Bold.ttf',
         'nunito': 'nunito/Nunito-900.ttf'}


def run(*a):
    subprocess.run(['magick', *map(str, a)], check=True)


def plaque(out, text, font, tmp):
    w, h = SIZE[0] * X, SIZE[1] * X
    b = BORDER * X
    run('-size', f'{w}x{h}', 'xc:none', '-fill', 'white', '-draw', f'roundrectangle 0,0 {w - 1},{h - 1} {2 * X + 2},{2 * X + 2}',
        '-fill', 'black', '-draw', f'roundrectangle {b},{b} {w - 1 - b},{h - 1 - b} {X + 2},{X + 2}', f'{tmp}/base.png')
    run('-background', 'none', '-font', font, '-pointsize', 200, '-fill', 'white', f'label:{text}', '-trim', '+repage', f'{tmp}/text0.png')
    run(f'{tmp}/text0.png', '-filter', 'Lanczos', '-resize', f'{TEXT_BOX[0]}x{TEXT_BOX[1]}>', f'{tmp}/text.png')
    tw, th = map(int, subprocess.run(['magick', 'identify', '-format', '%w %h', f'{tmp}/text.png'], capture_output=True, text=True, check=True).stdout.split())
    run(f'{tmp}/base.png', f'{tmp}/text.png', '-geometry', f'+{(w - tw) // 2}+{TEXT_CY - th // 2}', '-compose', 'over', '-composite', out)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--out', required=True)
    ap.add_argument('--lang', default='ru')
    ap.add_argument('--fonts', default='build/fonts')
    ap.add_argument('--preview', help='a folder for pictures of the whole plaque')
    a = ap.parse_args()
    tmp = os.path.join(a.out, '_labels_tmp')
    os.makedirs(tmp, exist_ok=True)
    for name, rel in FONTS.items():
        full = os.path.join(tmp, f'plaque_{a.lang}_{name}.png')
        plaque(full, TEXTS[a.lang], os.path.join(a.fonts, rel), tmp)
        done = set()
        for (x, y, w, h, key) in PIECES[a.lang]:
            if (w, h, key) in done:
                continue
            done.add((w, h, key))
            run(full, '-crop', f'{w * X}x{h * X}+{(x - ORIGIN[0]) * X}+{(y - ORIGIN[1]) * X}', '+repage', os.path.join(a.out, f'obj0_{w}x{h}_{key}@labels.{name}.png'))
        print(name, len(done), 'pictures')
        if a.preview:
            run(full, '-filter', 'point', '-resize', '50%', os.path.join(a.preview, f'dial_label_{a.lang}_{name}.png'))
    for f in os.listdir(tmp):
        os.remove(os.path.join(tmp, f))
    os.rmdir(tmp)


if __name__ == '__main__':
    main()
