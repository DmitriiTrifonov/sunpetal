#!/usr/bin/env python3
"""DS area models and PS2 scene models as textured triangles for the pairing page (`match_page.py`).

`ds_scene(b)`: a BMD0 file with its TEX0 block -> {'parts': [{tex, pos, uv, col, n}], 'textures': [png data URL]}.
`ps2_scene(export, id)`: a model of `sunpetal-ee`'s export -> the same. Positions are in model units (the DS
model's space; the PS2 scene's units), three vertices a triangle; uv in parts of the picture, v down; colours
0-255 (PS2: 128 is full). Arrays are base64 (float32, float32, uint8). Reads only.
"""
import base64
import json
import os
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, '..', 'ds'))
sys.path.insert(0, os.path.join(HERE, '..', 'ps2'))
sys.path.insert(0, os.path.join(HERE, '..'))
import align_area as aa
import lf
import nsbmd
from nsbmd import u16, u32, sx


def url(png):
    return 'data:image/png;base64,' + base64.b64encode(png).decode()


def b64(fmt, values):
    return base64.b64encode(struct.pack('<%d%s' % (len(values), fmt), *values)).decode()


# ---- DS textures (TEX0) ---------------------------------------------------------------------------

def rgb555(v):
    return ((v & 31) * 255 // 31, (v >> 5 & 31) * 255 // 31, (v >> 10 & 31) * 255 // 31)


def tex0(b):
    """The TEX0 block of a BMD0 file: ({texture name: (param, data offset)}, {palette name: offset}, block)."""
    for k in range(u16(b, 14)):
        t = u32(b, 16 + 4 * k)
        if b[t:t + 4] != b'TEX0':
            continue
        blk = {'tex': t + u32(b, t + 0x14), 'cmp': t + u32(b, t + 0x24), 'cmp_info': t + u32(b, t + 0x28), 'pal': t + u32(b, t + 0x38)}
        texs = {name: u32(b, e) for name, e in nsbmd.dictionary(b, t + u16(b, t + 0x0E))}
        pals = {name: u16(b, e) << 3 for name, e in nsbmd.dictionary(b, t + u32(b, t + 0x34))}
        return texs, pals, blk
    return {}, {}, None


def ds_texture(b, param, pal_off, blk):
    """A DS texture as (w, h, rgba), or None for a format with no palette given."""
    off, w, h = (param & 0xFFFF) << 3, 8 << (param >> 20 & 7), 8 << (param >> 23 & 7)
    fmt, c0 = param >> 26 & 7, param >> 29 & 1
    pal = lambda i: rgb555(u16(b, blk['pal'] + pal_off + 2 * i))
    out = bytearray(4 * w * h)
    if fmt == 7:
        for i in range(w * h):
            v = u16(b, blk['tex'] + off + 2 * i)
            out[4 * i:4 * i + 4] = bytes(rgb555(v) + (255 if v >> 15 else 0,))
    elif fmt == 5:
        for by in range(h // 4):
            for bx in range(w // 4):
                k = by * (w // 4) + bx
                bits = u32(b, blk['cmp'] + off + 4 * k)
                info = u16(b, blk['cmp_info'] + off // 2 + 2 * k)
                base, mode = blk['pal'] + pal_off + (info & 0x3FFF) * 4, info >> 14
                c = [rgb555(u16(b, base + 2 * i)) for i in range(2)] + [None, None]
                mix = lambda p, q: tuple((c[0][i] * p + c[1][i] * q) // (p + q) for i in range(3))
                if mode == 0:
                    c[2] = rgb555(u16(b, base + 4))
                elif mode == 1:
                    c[2] = mix(1, 1)
                elif mode == 2:
                    c[2], c[3] = rgb555(u16(b, base + 4)), rgb555(u16(b, base + 6))
                else:
                    c[2], c[3] = mix(5, 3), mix(3, 5)
                for y in range(4):
                    for x in range(4):
                        v = c[bits >> (2 * (4 * y + x)) & 3]
                        o = 4 * ((by * 4 + y) * w + bx * 4 + x)
                        out[o:o + 4] = bytes(v + (255,)) if v else b'\0\0\0\0'
    else:
        bpp = {1: 8, 2: 2, 3: 4, 4: 8, 6: 8}.get(fmt)
        if bpp is None or pal_off is None:
            return None
        for i in range(w * h):
            v = b[blk['tex'] + off + i * bpp // 8] >> (i * bpp % 8) & ((1 << bpp) - 1)
            if fmt == 1:
                out[4 * i:4 * i + 4] = bytes(pal(v & 31) + ((v >> 5) * 255 // 7,))
            elif fmt == 6:
                out[4 * i:4 * i + 4] = bytes(pal(v & 7) + ((v >> 3) * 255 // 31,))
            else:
                out[4 * i:4 * i + 4] = bytes(pal(v) + (0 if c0 and v == 0 else 255,))
    return w, h, bytes(out)


# ---- DS geometry ----------------------------------------------------------------------------------

def ds_triangles(b, at, size, k):
    """A display list's triangles with what the vertices carry: [[(pos, (s, t) in texels, (r, g, b)) x3]]."""
    tris, verts, mode = [], [], None
    pos, st, col = (0.0, 0.0, 0.0), (0.0, 0.0), (255, 255, 255)

    def vertex(p):
        verts.append((p, st, col))
        n = len(verts)
        if mode == 0 and n % 3 == 0:
            tris.append(verts[-3:])
        elif mode == 1 and n % 4 == 0:
            q = verts[-4:]
            tris.extend(([q[0], q[1], q[2]], [q[0], q[2], q[3]]))
        elif mode == 2 and n >= 3:
            tris.append(verts[-3:])
        elif mode == 3 and n >= 4 and n % 2 == 0:
            q = verts[-4:]
            tris.extend(([q[0], q[1], q[2]], [q[2], q[1], q[3]]))
    for c, w in nsbmd.decode_dl(b, at, size):
        if c == 0x40:
            mode, verts = w[0] & 3, []
        elif c == 0x41:
            mode, verts = None, []
        elif c == 0x20:
            col = rgb555(w[0])
        elif c == 0x21:
            col = (255, 255, 255)    # lit by its normal in the game: drawn white here
        elif c == 0x22:
            st = (sx(w[0], 16) / 16, sx(w[0] >> 16, 16) / 16)
        elif c == 0x23:
            pos = (sx(w[0], 16) * k, sx(w[0] >> 16, 16) * k, sx(w[1], 16) * k)
            vertex(pos)
        elif c == 0x24:
            pos = tuple(sx(w[0] >> (10 * i), 10) * k * 64 for i in range(3))
            vertex(pos)
        elif c == 0x25:
            pos = (sx(w[0], 16) * k, sx(w[0] >> 16, 16) * k, pos[2])
            vertex(pos)
        elif c == 0x26:
            pos = (sx(w[0], 16) * k, pos[1], sx(w[0] >> 16, 16) * k)
            vertex(pos)
        elif c == 0x27:
            pos = (pos[0], sx(w[0], 16) * k, sx(w[0] >> 16, 16) * k)
            vertex(pos)
        elif c == 0x28:
            pos = tuple(pos[i] + sx(w[0] >> (10 * i), 10) * k for i in range(3))
            vertex(pos)
    return tris


def ds_scene(b):
    m = nsbmd.parse(b)[0]
    texs, pals, blk = tex0(b)
    textures, index, parts = [], {}, {}
    for d in m.draws:
        mat, s = m.materials[d['mat']], m.shapes[d['shape']]
        w = m.nodes[d['node']]['world']
        key = (mat.get('tex'), mat.get('pal'))
        if key not in index:
            index[key] = -1
            if blk and key[0] in texs:
                t = ds_texture(b, texs[key[0]], pals.get(key[1]), blk)
                if t:
                    index[key] = len(textures)
                    textures.append({'url': url(lf.png_bytes(*t)), 'w': t[0], 'h': t[1], 'name': key[0]})
        ti = index[key]
        tw, th = (textures[ti]['w'], textures[ti]['h']) if ti >= 0 else (1, 1)
        part = parts.setdefault((ti, m.nodes[d['node']]['name']), {'tex': ti, 'node': m.nodes[d['node']]['name'], 'pos': [], 'uv': [], 'col': []})
        for tri in ds_triangles(b, s['dl'][0], s['dl'][1], m.pos_scale / 4096):
            for p, st, col in tri:
                part['pos'].extend(nsbmd.apply(w, p))
                part['uv'].extend((st[0] / tw, st[1] / th))
                part['col'].extend(col)
    return {'name': m.name, 'textures': textures, 'parts': [pack(p) for p in parts.values() if p['pos']]}


def pack(p):
    return {**{k: v for k, v in p.items() if k not in ('pos', 'uv', 'col')}, 'n': len(p['pos']) // 3,
            'pos': b64('f', p['pos']), 'uv': b64('f', p['uv']), 'col': b64('B', p['col'])}


# ---- PS2 ------------------------------------------------------------------------------------------

def ps2_scene(export, mid):
    j = json.load(open(f'{export}/models/{mid}.json'))
    raw = open(f'{export}/models/{mid}.vtx.bin', 'rb').read()
    worlds = []
    for o in j['objects']:
        ws = {}

        def world(i, joints=o['joints'], ws=ws):
            if i in ws:
                return ws[i]
            key = joints[i]['init_key']
            r, t = aa.rot(key[3:6]), key[0:3]
            p = joints[i]['parent']
            if p is not None and p >= 0:
                pr, pt = world(p)
                t = [sum(pr[a][c] * t[c] for c in range(3)) + pt[a] for a in range(3)]
                r = [[sum(pr[a][c] * r[c][bb] for c in range(3)) for bb in range(3)] for a in range(3)]
            ws[i] = (r, t)
            return ws[i]
        worlds.append([world(i) for i in range(len(o['joints']))])
    k = j['scene_scale']
    textures, index, parts = [], {}, {}
    for s in j['series']:
        mat = j['materials'][s['material']]
        t = mat.get('texture')
        if t not in index:
            index[t] = -1
            path = f'{export}/textures/{t}.png' if t else None
            if path and os.path.exists(path):
                png = open(path, 'rb').read()
                w, h = struct.unpack('>II', png[16:24])
                index[t] = len(textures)
                textures.append({'url': url(png), 'w': w, 'h': h, 'name': t})
        part = parts.setdefault((index[t], s['pass']), {'tex': index[t], 'pass': s['pass'], 'pos': [], 'uv': [], 'col': []})
        ws = worlds[s['object']]
        vs = []
        for i in range(s['first_vertex'], s['first_vertex'] + s['count']):
            v = struct.unpack_from('<6f4B2f4B', raw, 40 * i)
            slot = v[12]
            bone = s['bones'][slot] if slot < len(s['bones']) else (s['bones'][0] if s['bones'] else 0)
            r, tr = ws[bone] if bone < len(ws) else ws[0]
            vs.append((tuple((r[a][0] * v[0] + r[a][1] * v[1] + r[a][2] * v[2] + tr[a]) * k for a in range(3)), v[10:12], v[6:9]))
        n = len(vs)
        if s['prim_type'] == 'strip':
            idx = [(i, i + 1, i + 2) for i in range(n - 2)]
        elif s['prim_type'] == 'list':
            idx = [(i, i + 1, i + 2) for i in range(0, n - 2, 3)]
        elif s['prim_type'] == 'fan':
            idx = [(0, i, i + 1) for i in range(1, n - 1)]
        else:
            idx = []
        for tri in idx:
            if len({vs[x][0] for x in tri}) < 3:
                continue
            for x in tri:
                part['pos'].extend(vs[x][0])
                part['uv'].extend(vs[x][1])
                part['col'].extend(vs[x][2])
    order = sorted(parts.values(), key=lambda p: p['pass'])
    return {'name': mid, 'textures': textures, 'parts': [pack(p) for p in order if p['pos']]}
