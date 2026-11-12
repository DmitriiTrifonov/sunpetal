#!/usr/bin/env python3
"""Scenes shot without and with the PS2 models, and compared (docs/ULTIMATE-IMPORT.md, У4.5 and У4.11).

    sweep_scenes.py ROM.nds SAVE AREAS_DIR OUT_DIR SCENE[,SCENE...] [--jobs N]

SAVE: a save with the requests open (`build/full100.sav`; copied, not written). AREAS_DIR: what
`build_areas.py` made (`models`, `pack`). For each scene (`C02_01`): `sunpetal-debug --goto SCENE --shot`
twice -> OUT_DIR/<scene>_ds.png and _ps.png, then `compare_shots.py` on every pair. Exit 1 on a mismatch.
"""
import os
import shutil
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import compare_shots

DEBUG = os.path.join(HERE, '..', '..', 'target', 'release', 'sunpetal-debug')


def shot(job):
    rom, save, areas, out, scene, kind = job
    own = os.path.join(out, f'{scene}_{kind}.sav')
    shutil.copyfile(save, own)
    env = dict(os.environ)
    env.pop('SUNPETAL_ULT_MODELS', None)
    env.pop('SUNPETAL_TEXTURE_PACK', None)
    if kind == 'ps':
        env['SUNPETAL_ULT_MODELS'] = os.path.join(areas, 'models')
        env['SUNPETAL_TEXTURE_PACK'] = os.path.join(areas, 'pack')
    png = os.path.join(out, f'{scene}_{kind}.png')
    r = subprocess.run([DEBUG, rom, '--save', own, '--goto', scene, '--shot', png], env=env, capture_output=True, text=True)
    os.remove(own)
    with open(os.path.join(out, f'{scene}_{kind}.log'), 'w') as f:
        f.write(r.stdout + r.stderr)
    return scene, kind, r.returncode == 0 and os.path.exists(png)


def main(argv):
    if len(argv) < 5:
        print(__doc__)
        return 2
    rom, save, areas, out = argv[0], argv[1], os.path.abspath(argv[2]), argv[3]
    scenes = argv[4].split(',')
    jobs = int(argv[argv.index('--jobs') + 1]) if '--jobs' in argv else 5
    os.makedirs(out, exist_ok=True)
    with ThreadPoolExecutor(jobs) as pool:
        got = list(pool.map(shot, [(rom, save, areas, out, s, k) for s in scenes for k in ('ds', 'ps')]))
    ok = {(s, k) for s, k, good in got if good}
    bad = 0
    for s in scenes:
        if (s, 'ds') not in ok or (s, 'ps') not in ok:
            print(f'{s}: no picture (see {out}/{s}_*.log)')
            continue
        changed, e = compare_shots.compare(os.path.join(out, s + '_ds.png'), os.path.join(out, s + '_ps.png'))
        good = changed <= 0.02 or e >= 0.8
        bad += not good
        print(f'{s}: changed {changed:.3f} edges {e:.3f}{"" if changed > 0.02 else " (not replaced)"}{"" if good else " MISMATCH"}')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
