#!/usr/bin/env python3
"""The PS2 models of `data/ult/areas.toml` as `SPM2` files and one texture pack (docs/ULTIMATE-IMPORT.md, У4.5).

    build_areas.py ROM.nds EXPORT_DIR OUT_DIR [--areas FILE] [--only A46,A12...]

For every area marked `swap`: OUT_DIR/models/<Ann>.spm (bound to the DS model by its name and its file's size,
placed by the table's anchor and place) and the pictures of its materials in OUT_DIR/pack. `data/ult/areas_extra.toml` (beside the table) names the areas that several PS2 models stand for together:
<Ann>_1.spm, <Ann>_2.spm... Run the game with
`SUNPETAL_ULT_MODELS=OUT_DIR/models SUNPETAL_TEXTURE_PACK=OUT_DIR/pack`.

Reads the ROM (for the DS files' sizes) and the export; writes OUT_DIR only (game data: not for git).
"""
import os
import struct
import subprocess
import sys
import tomllib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..', 'ds'))
sys.path.insert(0, os.path.join(HERE, '..'))
import nsbmd
sys.path.insert(0, HERE)
import align_area as aa
from ndskit import rom as ndsrom


def main(argv):
    if len(argv) < 3:
        print(__doc__)
        return 2
    rom_path, export, out = argv[0], argv[1], argv[2]
    table = argv[argv.index('--areas') + 1] if '--areas' in argv else os.path.join(HERE, '..', '..', 'data', 'ult', 'areas.toml')
    only = argv[argv.index('--only') + 1].split(',') if '--only' in argv else None
    with open(table, 'rb') as f:
        areas = [a for a in tomllib.load(f)['area'] if a.get('status') == 'swap']
    extra = os.path.join(os.path.dirname(os.path.abspath(table)), 'areas_extra.toml')
    many = []
    if os.path.exists(extra):
        with open(extra, 'rb') as f:
            many = tomllib.load(f).get('area', [])
    areas = [a for a in areas if a['file'] not in {m['file'] for m in many}]
    for m in many:
        # several PS2 models for one DS area: each its own file, all bound to the one DS model
        _, _, anchor, mesh = aa.ds_points(rom_path, m['file'])
        for i, one in enumerate(m['models']):
            # the model mirrored in x, then turned about y by quarter turns, as tools/ult/match_page.html does
            turn = [[-1.0 if one.get('mirror') else 1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
            for _ in range(int(one.get('yaw', 0)) // 90 % 4):
                turn = [turn[2], turn[1], [-x for x in turn[0]]]
            rows, sh = aa.into_mesh(turn, list(one['shift']), mesh)
            areas.append({'file': m['file'], 'model': m['model'], 'ps2': one['ps2'], 'anchor': anchor,
                          'place': [x for r in rows for x in r] + sh, 'part': i + 1})
    rom = ndsrom.Rom(open(rom_path, 'rb').read())
    sizes = {p: struct.unpack_from('<I', b, 8)[0] for p, b in nsbmd.rom_models(rom) if '/Area/' in p}
    os.makedirs(os.path.join(out, 'models'), exist_ok=True)
    done = 0
    for a in areas:
        name = a['file'].split('/')[2]
        if only and name not in only:
            continue
        cmd = [sys.executable, os.path.join(HERE, 'ee2spm.py'), os.path.join(export, 'models'), a['ps2'],
               os.path.join(out, 'models', name + ('_%d' % a['part'] if 'part' in a else '') + '.spm'), '--bind', a['model'], '--size', str(sizes[a['file']]),
               '--anchor', str(a['anchor']), '--place', *(repr(float(x)) for x in a['place']), '--pack', os.path.join(out, 'pack')]
        r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode != 0:
            print('FAILED', name, r.stderr.strip().splitlines()[-1] if r.stderr.strip() else '')
            continue
        done += 1
    pics = len(os.listdir(os.path.join(out, 'pack'))) if os.path.isdir(os.path.join(out, 'pack')) else 0
    print(f'{done} models -> {out}/models, {pics} pictures -> {out}/pack')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
