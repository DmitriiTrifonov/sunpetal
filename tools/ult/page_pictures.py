#!/usr/bin/env python3
"""The pictures of the PS2 models' materials, read as the game's GS reads them (docs/ULTIMATE-IMPORT.md, U4.5).

    page_pictures.py EE_DIR OUT_DIR [--areas FILE] [--ids ID,ID...]

The export of sunpetal-ee (EE_DIR/out/export/v1) gives a material one upload of its scene's T-block. A PS2
texture is often a page put together by several uploads to one address with offsets (TRXPOS), and the
material's TEX0 names the whole page: uv are the page's. Here every PSMT8 upload of the T-block goes to a
model of GS memory (blocks of 16x16 in pages of 128x64), and a material's picture is its TEX0 rectangle read
back. OUT_DIR is a shadow export: models/<id>.json with the new picture ids (the .vtx.bin are links),
textures/ with the composed PNGs and links to the rest. Give OUT_DIR to build_areas.py as the export.

A bridge until the export does this itself (task X-32 of sunpetal-ee). Reads EE_DIR (its tools/gigtex.py,
extract/, out/assets/roles.tsv), writes OUT_DIR only (game data: not for git). By default the models of the
areas marked swap in data/ult/areas.toml and of areas_extra.toml.
"""
import sys, os, struct, json, hashlib, collections, tomllib
sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))

EE = EX = None
gigtex = None
BLK = None
roles = {}
files, mems = {}, {}


def setup(ee):
    global EE, EX, gigtex, BLK
    EE, EX = ee, ee + '/out/export/v1'
    sys.path.insert(0, EE + '/tools')
    import gigtex as g
    gigtex, BLK = g, g.BLK32
    for ln in open(EE + '/out/assets/roles.tsv').read().split('\n')[1:]:
        if ln:
            f, i, o, sz, r = ln.split('\t')[:5]
            roles[(f, int(i))] = (int(o), int(sz), r)


def load(fn):
    if fn not in files:
        p = EE + '/extract/data/' + fn
        files[fn] = open(p if os.path.exists(p) else EE + '/extract/event/' + fn, 'rb').read()
    return files[fn]


def block(bp, bw, x, y):
    # PSMT8: pages of 128x64, 32 blocks of 16x16
    page = (y // 64) * max(bw // 2, 1) + x // 128
    return bp + 32 * page + BLK[(y % 64) // 16][(x % 128) // 16]


def memory(fn, rec_i):
    """(blocks of PSMT8 pixels by block number, the uploads) after the record's T-block."""
    if (fn, rec_i) in mems:
        return mems[(fn, rec_i)]
    o, sz, role = roles[(fn, rec_i)]
    rec = load(fn)[o:o + sz]
    t = struct.unpack_from('<I', rec, 0x1c if role == 'Scene3D' else 4)[0]
    ups = []
    if t:
        n = struct.unpack_from('<I', rec, t)[0]
        for e in range(n):
            eo, es = struct.unpack_from('<II', rec, t + 4 + 8 * e)
            ups += gigtex.uploads(rec, t + eo, t + eo + es)
    mem = {}
    for u in ups:
        if u.psm != 0x13 or u.w * u.h != len(u.data):
            continue
        for y in range(u.h):
            for x in range(u.w):
                X, Y = u.dsax + x, u.dsay + y
                b = mem.setdefault(block(u.dbp, u.dbw, X, Y), bytearray(256))
                b[(Y % 16) * 16 + X % 16] = u.data[y * u.w + x]
    mems[(fn, rec_i)] = (mem, ups)
    return mem, ups


def main(out, ids):
    os.makedirs(out + '/models', exist_ok=True)
    os.makedirs(out + '/textures', exist_ok=True)
    st = collections.Counter()
    for mid in ids:
        j = json.load(open(f'{EX}/models/{mid}.json'))
        occ = None
        for oc in j['occurrences']:
            if '#' in oc.split('@')[0]:
                fn, ri = oc.split('#')[0], int(oc.split('#')[1].split('@')[0])
                if roles.get((fn, ri), (0, 0, None))[2] in ('Scene3D', 'Chara'):
                    occ = (fn, ri)
                    break
        for m in j['materials']:
            t = m.get('texture')
            if t and os.path.exists(f'{EX}/textures/{t}.png') and not os.path.exists(f'{out}/textures/{t}.png'):
                os.symlink(f'{EX}/textures/{t}.png', f'{out}/textures/{t}.png')
        if occ:
            mem, ups = memory(*occ)
            for m in j['materials']:
                if m['psm'] != 0x13 or not int(m['prim'], 16) & 0x10:
                    continue
                st['materials'] += 1
                w, h = 1 << m['tw'], 1 << m['th']
                tbw = (int(m['tex0'], 16) >> 14) & 63
                px, miss = bytearray(w * h), 0
                for y in range(h):
                    for x in range(w):
                        b = mem.get(block(m['tbp0'], tbw, x, y))
                        if b is None:
                            miss += 1
                        else:
                            px[y * w + x] = b[(y % 16) * 16 + x % 16]
                cl = [u for u in ups if u.psm == 0 and u.w == 16 and u.h == 16 and u.clut_block() == m['cbp']]
                if not cl or miss == w * h:
                    st['no clut' if not cl else 'nothing loaded'] += 1
                    continue
                st['holes'] += miss > 0
                name = hashlib.sha1(bytes(px)).hexdigest()[:10] + '_' + hashlib.sha1(cl[-1].data).hexdigest()[:8]
                st['same' if name == m.get('texture') else ('new' if m.get('texture') else 'was none')] += 1
                if name != m.get('texture'):
                    p = f'{out}/textures/{name}.png'
                    if not os.path.exists(p):
                        c = gigtex.clut_colors(cl[-1])
                        gigtex.write_png(p, w, h, b''.join(bytes(c[i]) for i in px))
                    m['texture'] = name
        else:
            st['model with no scene'] += 1
        json.dump(j, open(f'{out}/models/{mid}.json', 'w'))
        if not os.path.exists(f'{out}/models/{mid}.vtx.bin'):
            os.symlink(f'{EX}/models/{mid}.vtx.bin', f'{out}/models/{mid}.vtx.bin')
    print(dict(st))


def ids_of(table):
    with open(table, 'rb') as f:
        ids = {a['ps2'] for a in tomllib.load(f)['area'] if a.get('status') == 'swap'}
    extra = os.path.join(os.path.dirname(os.path.abspath(table)), 'areas_extra.toml')
    if os.path.exists(extra):
        with open(extra, 'rb') as f:
            ids |= {one['ps2'] for a in tomllib.load(f).get('area', []) for one in a['models']}
    return sorted(ids)


if __name__ == '__main__':
    a = sys.argv[1:]
    if len(a) < 2:
        print(__doc__)
        sys.exit(2)
    setup(os.path.abspath(os.path.expanduser(a[0])))
    table = a[a.index('--areas') + 1] if '--areas' in a else os.path.join(HERE, '..', '..', 'data', 'ult', 'areas.toml')
    main(a[1], a[a.index('--ids') + 1].split(',') if '--ids' in a else ids_of(table))
