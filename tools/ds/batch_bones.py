#!/usr/bin/env python3
"""Names the bone of each batch of a frame (docs/ULTIMATE-IMPORT.md, У3.5).

    printf 'advance 5\\nbatches\\nquit\\n' | sunpetal-debug ROM --save S --goto C02_01 --repl | batch_bones.py ROM.nds

`batches` of sunpetal-debug lists the frame's runs of polygons as (model, mesh, slot): the model and mesh
the game's DMA came out of, and the matrix-stack slot of the last MTX_RESTORE before them. The slot is the
node whose matrix the game stored there (`nsbmd.Model.stack_nodes`): this prints the node (bone) name.
Reads the JSON line `{"meshes": [...]}` from stdin.
"""
import json
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ndskit import rom as ndsrom
import nsbmd


def main(argv):
    if not argv:
        print(__doc__)
        return 2
    rows = None
    for line in sys.stdin:
        line = line.strip()
        if line.startswith('{') and '"meshes"' in line:
            rows = json.loads(line)['meshes']
    if rows is None:
        print('no "meshes" line on stdin', file=sys.stderr)
        return 1
    rom = ndsrom.Rom(open(argv[0], 'rb').read())
    models = {}
    for _, b in nsbmd.rom_models(rom):
        try:
            for m in nsbmd.parse(b):
                models.setdefault(m.name, m)
        except (nsbmd.NsbmdError, IndexError):
            pass
    unresolved = 0
    for r in rows:
        m = models.get(r['model'])
        node = None
        if m is not None and r['slot'] != 255:
            stored, mixes = m.stack_nodes()
            node = stored.get(r['slot'])
        if node is None:
            # a mesh's own node when the batch comes without a restore (slot 255)
            if m is not None:
                node = next((x['node'] for x in m.mesh_nodes() if x['shape'] == r['mesh']), None)
            unresolved += r['slot'] != 255
        name = m.nodes[node]['name'] if m is not None and node is not None else '?'
        print(f"{r['model']:28} mesh {r['mesh']:3} {r['name']:14} slot {r['slot']:3} -> {name:20} runs {r['runs']:3} polys {r['polys']}")
    print(f'{len(rows)} rows, {unresolved} with a slot that names no node', file=sys.stderr)
    return 1 if unresolved else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
