#!/usr/bin/env python3
"""Every DS area model against every PS2 scene model, as `align_area.py`'s first pass does one
(docs/ULTIMATE-IMPORT.md, У4.1 and У4.2): is the DS model a PS2 one shifted, and by how much.

    align_all.py ROM.nds EXPORT_DIR OUT.toml [--eps E] [--jobs N] [--only A46,A00...] [--all-models]

Writes OUT.toml (`data/ult/areas.toml`): per DS area file the PS2 model, the share of the DS vertices with a
PS2 vertex within `--eps` (0.3), the shift into the DS model's space, the anchor mesh and the place for
`ee2spm.py` (in that mesh's display-list units). An area with no model at a share of 0.5 is marked `ds-only`
with its best guess. `--all-models`: not the scenes' GIG slots alone but every model of the export. Twins (PS2 models at the same share) are listed in `also`.

Reads the ROM and the export; writes OUT.toml and a cache of the PS2 clouds beside it in `build/ult/`.
"""
import glob
import json
import multiprocessing
import os
import pickle
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, '..', 'ds'))
sys.path.insert(0, os.path.join(HERE, '..'))
import align_area as aa
import nsbmd
from ndskit import rom as ndsrom

CLOUDS = None


def ps_clouds(export, cache, every=False):
    """{model id: sorted unique vertices} of the scenes' GIG models, or of `every` model of the export
    (cached: the export's manifest names the cache)."""
    stamp = os.path.getmtime(os.path.join(export, 'manifest.json'))
    if os.path.exists(cache):
        with open(cache, 'rb') as f:
            got = pickle.load(f)
        if got.get('stamp') == stamp:
            return got['clouds']
    ids = set()
    for f in sorted(glob.glob(export + '/scenes/*.json')):
        for s in json.load(open(f))['scene3d']:
            ids.update(mid for slot, mid in s['models'].items() if slot == 'GIG')
    if every:
        ids = {os.path.basename(f)[:-5] for f in glob.glob(export + '/models/*.json')}
    clouds = {}
    for mid in sorted(ids):
        try:
            ps = sorted(set(aa.ps2_points(export, mid)))
        except (OSError, KeyError, IndexError, struct.error):
            continue
        if len(ps) >= 30:
            clouds[mid] = ps
    os.makedirs(os.path.dirname(cache), exist_ok=True)
    with open(cache, 'wb') as f:
        pickle.dump({'stamp': stamp, 'clouds': clouds}, f)
    return clouds


def peak(ds, ps, sample=30, step=0.5):
    """A quick look: the most votes one shift gets from `sample` DS vertices."""
    votes = {}
    for d in aa.thin(ds, sample):
        for p in ps:
            k = (round((d[0] - p[0]) / step), round((d[1] - p[1]) / step), round((d[2] - p[2]) / step))
            votes[k] = votes.get(k, 0) + 1
    return max(votes.values())


def one(job):
    rom_path, path, cache, eps = job
    global CLOUDS
    if CLOUDS is None:
        with open(cache, 'rb') as f:
            CLOUDS = pickle.load(f)['clouds']
    try:
        name, ds, anchor, mesh = aa.ds_points(rom_path, path)
    except (nsbmd.NsbmdError, struct.error, IndexError, SystemExit) as e:
        return {'file': path, 'error': str(e)}
    ds = sorted(set(ds))
    if len(ds) < 30:
        return {'file': path, 'model': name, 'vertices': len(ds), 'error': 'too few vertices'}
    found = []
    for mid, ps in CLOUDS.items():
        n = min(30, len(ds))
        if peak(ds, ps) < max(4, n // 5):
            continue
        share, dist, shift = aa.same_model(ds, ps, eps)
        found.append((share, -dist, mid, shift))
    found.sort(reverse=True)
    out = {'file': path, 'model': name, 'vertices': len(ds), 'anchor': anchor}
    if found:
        share, dist, mid, shift = found[0]
        rows, sh = aa.into_mesh([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], shift, mesh)
        out.update(ps2=mid, share=share, middle=-dist, shift=shift, place=[x for r in rows for x in r] + sh,
                   also=[m for s, _, m, _ in found[1:] if s >= share - 0.01][:8],
                   next=next((s for s, _, m, _ in found[1:] if s < share - 0.01), 0.0))
    return out


def main(argv):
    if len(argv) < 3:
        print(__doc__)
        return 2
    rom_path, export, out = argv[0], argv[1], argv[2]
    eps = float(argv[argv.index('--eps') + 1]) if '--eps' in argv else 0.3
    jobs = int(argv[argv.index('--jobs') + 1]) if '--jobs' in argv else max(1, (os.cpu_count() or 2) - 2)
    only = argv[argv.index('--only') + 1].split(',') if '--only' in argv else None
    every = '--all-models' in argv
    cache = os.path.join('build', 'ult', 'ps2_clouds_all.pickle' if every else 'ps2_clouds.pickle')
    clouds = ps_clouds(export, cache, every)
    print(len(clouds), 'PS2 scene models', file=sys.stderr)
    rom = ndsrom.Rom(open(rom_path, 'rb').read())
    paths = sorted(p for p, _ in nsbmd.rom_models(rom) if '/Area/' in p and (not only or any('/' + a + '/' in p for a in only)))
    with multiprocessing.Pool(jobs) as pool:
        rows = []
        for r in pool.imap(one, [(rom_path, p, cache, eps) for p in paths]):
            rows.append(r)
            print(r['file'], r.get('ps2', '-'), round(r.get('share', 0.0), 3), r.get('error', ''), file=sys.stderr, flush=True)
    num = lambda v: ', '.join(f'{x:.6g}' for x in v)
    lines = ['# DS area models and the PS2 scene models they were made from (tools/ult/align_all.py; do not edit by hand).',
             f'# share: the part of the DS vertices with a PS2 vertex within {eps}; shift: PS2 model -> DS model space;',
             "# place: PS2 model -> the anchor mesh's display-list units (for ee2spm.py --anchor --place).", '']
    for r in rows:
        lines.append('[[area]]')
        lines.append(f'file = "{r["file"]}"')
        if 'model' in r:
            lines.append(f'model = "{r["model"]}"')
            lines.append(f'vertices = {r["vertices"]}')
        if 'error' in r:
            lines.append(f'status = "error"')
            lines.append(f'note = "{r["error"]}"')
        elif 'ps2' not in r:
            lines.append('status = "ds-only"')
        else:
            lines.append(f'status = "{"swap" if r["share"] >= 0.5 else "ds-only"}"')
            lines.append(f'ps2 = "{r["ps2"]}"')
            lines.append(f'share = {r["share"]:.3f}')
            lines.append(f'middle = {r["middle"]:.4f}')
            lines.append(f'next = {r["next"]:.3f}')
            if r['also']:
                lines.append('also = [' + ', '.join(f'"{m}"' for m in r['also']) + ']')
            lines.append(f'shift = [{num(r["shift"])}]')
            lines.append(f'anchor = {r["anchor"]}')
            lines.append(f'place = [{num(r["place"])}]')
        lines.append('')
    os.makedirs(os.path.dirname(out) or '.', exist_ok=True)
    with open(out, 'w') as f:
        f.write('\n'.join(lines))
    ok = sum(1 for r in rows if r.get('share', 0.0) >= 0.5)
    print(f'{len(rows)} areas: {ok} swap, {len(rows) - ok} ds-only or error -> {out}')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
