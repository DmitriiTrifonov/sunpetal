#!/usr/bin/env python3
"""A page to pair DS areas with PS2 scene models by eye (docs/ULTIMATE-IMPORT.md, У4.2).

    match_page.py ROM.nds EXPORT_DIR OUT_DIR [--areas data/ult/areas.toml]

Writes OUT_DIR/index.html, OUT_DIR/data.js (the vertices of every DS area model and every PS2 model of the
export) and, for the 3D view, OUT_DIR/areas/<Ann>.js, models/<id>.js (triangles; `scene3d.py`) and
tex/<id>.js (the PS2 pictures): game data, not for git — keep OUT_DIR under build/ (about 250 MB). Open index.html in a browser: the DS area in red,
a PS2 model in blue over it; move, turn and mirror the PS2 model, see the share of DS vertices it meets, let
the page rank all the PS2 models for the area, fly through both in 3D (one view or two side by side with one camera), walk the PS2 scenes' records, and collect the pairs as lines
for `data/ult/areas_extra.toml`. Needs `build/ult/ps2_clouds_all.pickle` (`align_all.py --all-models`).
"""
import base64
import glob
import json
import os
import pickle
import struct
import sys
import tomllib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, '..', 'ds'))
sys.path.insert(0, os.path.join(HERE, '..'))
import align_area as aa
import nsbmd
import scene3d
from ndskit import rom as ndsrom


def pack(pts):
    return base64.b64encode(struct.pack('<%df' % (3 * len(pts)), *[x for p in pts for x in p])).decode()


def main(argv):
    if len(argv) < 3:
        print(__doc__)
        return 2
    rom_path, export, out = argv[:3]
    table = argv[argv.index('--areas') + 1] if '--areas' in argv else os.path.join(HERE, '..', '..', 'data', 'ult', 'areas.toml')
    with open(table, 'rb') as f:
        rows = {a['file']: a for a in tomllib.load(f)['area']}
    with open(os.path.join('build', 'ult', 'ps2_clouds_all.pickle'), 'rb') as f:
        clouds = pickle.load(f)['clouds']
    rom = ndsrom.Rom(open(rom_path, 'rb').read())
    os.makedirs(out, exist_ok=True)
    areas = []
    for path, b in sorted(nsbmd.rom_models(rom)):
        if '/Area/' not in path:
            continue
        try:
            m = nsbmd.parse(b)[0]
        except (nsbmd.NsbmdError, struct.error, IndexError):
            continue
        pts = set()
        for d in m.draws:
            s = m.shapes[d['shape']]
            w = m.nodes[d['node']]['world']
            pts.update(nsbmd.apply(w, v) for _, tri in nsbmd.triangles(b, s['dl'][0], s['dl'][1], m.pos_scale) for v in tri)
        os.makedirs(os.path.join(out, 'areas'), exist_ok=True)
        with open(os.path.join(out, 'areas', path.split('/')[2] + '.js'), 'w') as f:
            f.write('window.SCENE(%s, %s);\n' % (json.dumps('ds:' + path.split('/')[2]), json.dumps(scene3d.ds_scene(b))))
        r = rows.get(path, {})
        areas.append({'id': path.split('/')[2], 'file': path, 'model': m.name, 'status': r.get('status', '?'),
                      'ps2': r.get('ps2'), 'also': r.get('also', []), 'share': r.get('share'), 'loose': r.get('loose'), 'shift': r.get('shift'),
                      'n': len(pts), 'pts': pack(sorted(pts))})
    where = {}
    scenes = []
    for f in sorted(glob.glob(export + '/scenes/*.json')):
        j = json.load(open(f))
        recs = []
        for s in j['scene3d']:
            recs.append({'record': s['record'], 'models': s['models']})
            for slot, mid in s['models'].items():
                where.setdefault(mid, []).append(f"{j['scene']}#{s['record']}:{slot}")
        scenes.append({'scene': j['scene'], 'records': recs})
    models = [{'id': mid, 'n': len(ps), 'where': where.get(mid, []), 'pts': pack(ps)} for mid, ps in sorted(clouds.items())]
    os.makedirs(os.path.join(out, 'models'), exist_ok=True)
    os.makedirs(os.path.join(out, 'tex'), exist_ok=True)
    seen = set()
    for mid in sorted(clouds):
        sc = scene3d.ps2_scene(export, mid)
        for t in sc['textures']:
            # the pictures apart, each once: the models share them
            if t['name'] not in seen:
                seen.add(t['name'])
                with open(os.path.join(out, 'tex', t['name'] + '.js'), 'w') as f:
                    f.write('window.SCENE(%s, %s);\n' % (json.dumps('tex:' + t['name']), json.dumps(t['url'])))
            del t['url']
        with open(os.path.join(out, 'models', mid + '.js'), 'w') as f:
            f.write('window.SCENE(%s, %s);\n' % (json.dumps('ps2:' + mid), json.dumps(sc)))
    with open(os.path.join(out, 'data.js'), 'w') as f:
        f.write('window.DATA = ' + json.dumps({'areas': areas, 'models': models, 'scenes': scenes}) + ';\n')
    with open(os.path.join(HERE, 'match_page.html')) as f:
        page = f.read()
    with open(os.path.join(out, 'index.html'), 'w') as f:
        f.write(page)
    size = os.path.getsize(os.path.join(out, 'data.js')) / 1e6
    print(f'{len(areas)} DS areas, {len(models)} PS2 models, {len(scenes)} scenes -> {out}/index.html (data.js {size:.1f} MB)')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
