#!/usr/bin/env python3
"""The DS game's 3D models (`.nsbmd`, BMD0 files with an MDL0 block), read from the ROM's files alone.

Per model: nodes (parent, local SRT matrix, world matrix), materials (polygon attributes, texture and
palette names), meshes ("shapes": display-list bytes, sizes) and the SBC program (which mesh is drawn
under which node and material). A display list is decoded to triangles by `triangles()`.

    nsbmd.py ROM.nds            counts for every model file (exit 1 if any file fails)
    nsbmd.py ROM.nds PATH       the model(s) of one file as JSON (`Model/Area/A46/A46_00.nsbmd.z`)
    nsbmd.py FILE.nsbmd         a plain (already decompressed) file as JSON

Units: positions are `fx32` / `fx16` (1/4096) as the file has them; the model's `pos_scale` (a power
of two as a plain number) is what the game multiplies positions by (POSSCALE). Matrices are 4x3
row-vector style lists `[[m00 m01 m02] [m10 ..] [m20 ..] [tx ty tz]]` in plain floats.
"""
import json
import os
import struct
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ndskit import lz, rom as ndsrom
import nitro


class NsbmdError(Exception):
    pass


def u8(b, o):
    return b[o]


def u16(b, o):
    return struct.unpack_from('<H', b, o)[0]


def s16(b, o):
    return struct.unpack_from('<h', b, o)[0]


def u32(b, o):
    return struct.unpack_from('<I', b, o)[0]


def s32(b, o):
    return struct.unpack_from('<i', b, o)[0]


def chk(b, o, n):
    if o < 0 or o + n > len(b):
        raise NsbmdError(f'read of {n} bytes at {o:#x} outside the file ({len(b):#x})')


def dictionary(b, at):
    """The NITRO dictionary at `at`: [(name, offset of the entry's data)]."""
    chk(b, at, 8)
    n = b[at + 1]
    entries = at + u16(b, at + 6)
    chk(b, entries, 4)
    unit = u16(b, entries)
    names = entries + 4 + n * unit
    chk(b, names, 16 * n)
    out = []
    for i in range(n):
        name = bytes(b[names + 16 * i:names + 16 * i + 16]).split(b'\0')[0].decode('latin1')
        out.append((name, entries + 4 + i * unit))
    return out


# ---- matrices: row vectors, [[a b c] [d e f] [g h i] [tx ty tz]] -----------------------------------

IDENT = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, 0.0]]


def mul(a, b):
    """`a` then `b` (row vectors: p * a * b)."""
    r = [[sum(a[i][k] * b[k][j] for k in range(3)) for j in range(3)] for i in range(3)]
    t = [sum(a[3][k] * b[k][j] for k in range(3)) + b[3][j] for j in range(3)]
    return r + [t]


def apply(m, p):
    return tuple(sum(p[k] * m[k][j] for k in range(3)) + m[3][j] for j in range(3))


def pivot_rotation(flags, a, b):
    """The rotation a pivot node stores as two numbers: one cell is +-1, the other four hold A, B."""
    p = flags >> 4 & 15
    if p > 8:
        raise NsbmdError(f'pivot cell {p}')
    one_r, one_c = divmod(p, 3)
    rows = [i for i in range(3) if i != one_r]
    cols = [j for j in range(3) if j != one_c]
    m = [[0.0] * 3 for _ in range(3)]
    m[one_r][one_c] = -1.0 if flags & 0x100 else 1.0
    m[rows[0]][cols[0]] = a
    m[rows[0]][cols[1]] = b
    m[rows[1]][cols[0]] = -b if flags & 0x200 else b
    m[rows[1]][cols[1]] = -a if flags & 0x400 else a
    return m


def node_srt(b, at):
    """A node's transform data at `at`: (flags, T, R 3x3, S, size of the data in bytes)."""
    chk(b, at, 4)
    flags = u16(b, at)
    o = at + 2
    r00 = s16(b, o) / 4096
    o += 2
    t = (0.0, 0.0, 0.0)
    if not flags & 1:
        chk(b, o, 12)
        t = tuple(s32(b, o + 4 * i) / 4096 for i in range(3))
        o += 12
    rot = [[1.0, 0, 0], [0, 1.0, 0], [0, 0, 1.0]]
    if not flags & 2:
        if flags & 8:
            chk(b, o, 4)
            rot = pivot_rotation(flags, s16(b, o) / 4096, s16(b, o + 2) / 4096)
            o += 4
        else:
            chk(b, o, 16)
            v = [s16(b, o + 2 * i) / 4096 for i in range(8)]
            rot = [[r00, v[0], v[1]], [v[2], v[3], v[4]], [v[5], v[6], v[7]]]
            o += 16
    s = (1.0, 1.0, 1.0)
    if not flags & 4:
        chk(b, o, 24)
        s = tuple(s32(b, o + 4 * i) / 4096 for i in range(3))
        o += 24
    return flags, t, rot, s, o - at


def local_matrix(t, rot, s):
    """Scale, then rotate, then translate (row vectors)."""
    r = [[rot[i][j] * s[i] for j in range(3)] for i in range(3)]
    return r + [list(t)]


# ---- SBC: the draw program -------------------------------------------------------------------------

def sbc_args(op, b, o):
    """The number of parameter bytes of SBC command `op` at `o` (just after the opcode)."""
    c, hi = op & 0x1F, op >> 5
    if op == 0xFF or c == 0x01 or c == 0x00:
        return 0
    if c == 0x02:
        return 2
    if c == 0x03:
        return 1
    if c in (0x04, 0x05):
        return 1
    if c == 0x06:
        return 3 + (1 if hi & 1 else 0) + (1 if hi & 2 else 0)
    if c in (0x07, 0x08):
        return 1 + (1 if hi & 1 else 0)
    if c == 0x09:
        chk(b, o, 2)
        return 2 + 3 * b[o + 1]
    if c == 0x0A:
        return 8
    if c == 0x0B:
        return 0
    if c in (0x0C, 0x0D):
        return 2
    raise NsbmdError(f'SBC opcode {op:#04x} at {o - 1:#x}')


def parse_sbc(b, at, end, n_nodes, n_mats, n_shapes):
    """The SBC program from `at` to its RET (0x01): a list of commands `{op, ...}` and the node tree
    it declares (`nodes[id] = {parent, ...}`), and the draws: (node, material, shape, restore index)."""
    cmds, draws = [], []
    parents = {}
    o = at
    node = mat = None
    restore = None
    while True:
        chk(b, o, 1)
        op = b[o]
        o += 1
        n = sbc_args(op, b, o)
        chk(b, o, n)
        a = list(b[o:o + n])
        c = op & 0x1F
        cmd = {'op': op, 'args': a}
        if c == 0x01 or op == 0xFF:
            cmds.append(cmd)
            o += n
            break
        if c == 0x02:
            node = a[0]
            if a[0] >= n_nodes:
                raise NsbmdError(f'SBC node {a[0]} of {n_nodes}')
        elif c == 0x03:
            restore = a[0]
        elif c == 0x04:
            mat = a[0]
            if a[0] >= n_mats:
                raise NsbmdError(f'SBC material {a[0]} of {n_mats}')
        elif c == 0x05:
            if a[0] >= n_shapes:
                raise NsbmdError(f'SBC shape {a[0]} of {n_shapes}')
            draws.append({'node': node, 'mat': mat, 'shape': a[0], 'restore': restore})
        elif c == 0x06:
            parents[a[0]] = a[1]
            cmd['node'], cmd['parent'] = a[0], a[1]
            if hi_of(op) & 1:
                cmd['restore'] = a[3]
            if hi_of(op) & 2:
                cmd['store'] = a[-1]
        cmds.append(cmd)
        o += n
    if o > end:
        raise NsbmdError(f'SBC runs to {o:#x}, past its block {end:#x}')
    return cmds, parents, draws


def hi_of(op):
    return op >> 5


# ---- model parts -----------------------------------------------------------------------------------

def parse_materials(b, at, n_mats):
    """The materials block at `at`: [{name, dif_amb, spe_emi, poly_attr, tex_image, flag, w, h, tex, pal}]."""
    chk(b, at, 4)
    tex_dict, pal_dict = u16(b, at), u16(b, at + 2)
    mats = dictionary(b, at + 4)
    names = {}
    for key, dic in (('tex', tex_dict), ('pal', pal_dict)):
        if not dic:
            continue
        for name, e in dictionary(b, at + dic):
            ofs, cnt = u16(b, e), b[e + 2]
            chk(b, at + ofs, cnt)
            for mid in b[at + ofs:at + ofs + cnt]:
                names.setdefault(mid, {})[key] = name
    out = []
    for i, (name, e) in enumerate(mats):
        m = at + u32(b, e)
        chk(b, m, 0x2C)
        out.append({
            'name': name,
            'dif_amb': u32(b, m + 4), 'spe_emi': u32(b, m + 8),
            'poly_attr': u32(b, m + 12), 'poly_attr_mask': u32(b, m + 16),
            'tex_image': u32(b, m + 20), 'tex_image_mask': u32(b, m + 24),
            'pltt_base': u16(b, m + 28), 'flag': u16(b, m + 30),
            'w': u16(b, m + 32), 'h': u16(b, m + 34),
            'tex': names.get(i, {}).get('tex'), 'pal': names.get(i, {}).get('pal'),
        })
    if len(out) != n_mats:
        raise NsbmdError(f'{len(out)} materials, header says {n_mats}')
    return out


def parse_shapes(b, at, n_shapes):
    """The meshes: [{name, flag, dl: (offset, size)}]; offsets are in the file."""
    out = []
    for name, e in dictionary(b, at):
        s = at + u32(b, e)
        chk(b, s, 16)
        dl, size = s + u32(b, s + 8), u32(b, s + 12)
        chk(b, dl, size)
        out.append({'name': name, 'flag': u32(b, s + 4), 'dl': (dl, size)})
    if len(out) != n_shapes:
        raise NsbmdError(f'{len(out)} shapes, header says {n_shapes}')
    return out


class Model:
    def __init__(self, b, name, at):
        """`at`: the model's offset in the file `b`."""
        self.name = name
        chk(b, at, 0x40)
        self.at = at
        self.size = u32(b, at)
        ofs_sbc, ofs_mat, ofs_shp = u32(b, at + 4), u32(b, at + 8), u32(b, at + 12)
        self.ofs_evp = u32(b, at + 16)
        self.sbc_type, self.scaling_rule, self.texmtx_mode, n_nodes = b[at + 20:at + 24]
        n_mats, n_shapes, self.first_unused_stack = b[at + 24], b[at + 25], b[at + 26]
        self.pos_scale = s32(b, at + 28) / 4096
        self.inv_pos_scale = s32(b, at + 32) / 4096
        self.n_vtx, self.n_poly, self.n_tri, self.n_quad = (u16(b, at + 36 + 2 * i) for i in range(4))
        self.box = {
            'min': [s16(b, at + 44 + 2 * i) / 4096 for i in range(3)],
            'size': [s16(b, at + 50 + 2 * i) / 4096 for i in range(3)],
            'pos_scale': s32(b, at + 56) / 4096,
        }
        # nodes
        self.nodes = []
        for nm, e in dictionary(b, at + 0x40):
            data = at + 0x40 + u32(b, e)    # node offsets count from the node dictionary
            flags, t, rot, s, _ = node_srt(b, data)
            self.nodes.append({'name': nm, 'flags': flags, 't': t, 'r': rot, 's': s,
                               'local': local_matrix(t, rot, s)})
        if len(self.nodes) != n_nodes:
            raise NsbmdError(f'{len(self.nodes)} nodes, header says {n_nodes}')
        self.materials = parse_materials(b, at + ofs_mat, n_mats)
        self.shapes = parse_shapes(b, at + ofs_shp, n_shapes)
        self.sbc, parents, self.draws = parse_sbc(b, at + ofs_sbc, at + ofs_mat, n_nodes, n_mats, n_shapes)
        for i, nd in enumerate(self.nodes):
            nd['parent'] = parents.get(i)          # None: no NODEDESC (root, or a skinning-only node)
        self._world()

    def _world(self):
        """World matrices: the chain of local matrices up the tree (a node with no declared parent is a root)."""
        done = {}

        def world(i, depth=0):
            if i in done:
                return done[i]
            if depth > len(self.nodes):
                raise NsbmdError('cycle in the node tree')
            nd = self.nodes[i]
            p = nd['parent']
            m = nd['local'] if p is None or p == i or p == 0xFF or p >= len(self.nodes) else mul(nd['local'], world(p, depth + 1))
            done[i] = m
            return m
        for i, nd in enumerate(self.nodes):
            nd['world'] = world(i)

    def as_json(self, b=None):
        return {
            'name': self.name, 'pos_scale': self.pos_scale, 'inv_pos_scale': self.inv_pos_scale,
            'sbc_type': self.sbc_type, 'scaling_rule': self.scaling_rule, 'texmtx_mode': self.texmtx_mode,
            'counts': {'vtx': self.n_vtx, 'poly': self.n_poly, 'tri': self.n_tri, 'quad': self.n_quad},
            'box': self.box,
            'nodes': [{'index': i, 'name': n['name'], 'parent': n['parent'], 'flags': n['flags'], 't': n['t'],
                       's': n['s'], 'local': n['local'], 'world': n['world']} for i, n in enumerate(self.nodes)],
            'materials': self.materials,
            'shapes': [{'index': i, 'name': s['name'], 'flag': s['flag'], 'dl_offset': s['dl'][0], 'dl_size': s['dl'][1]}
                       for i, s in enumerate(self.shapes)],
            'draws': self.draws,
        }


def parse(b):
    """The models of a BMD0 file `b`: [Model]."""
    if b[:4] != b'BMD0':
        raise NsbmdError('not a BMD0 file')
    n = u16(b, 14)
    out = []
    for k in range(n):
        blk = u32(b, 16 + 4 * k)
        chk(b, blk, 8)
        if b[blk:blk + 4] != b'MDL0':
            continue
        for name, e in dictionary(b, blk + 8):
            out.append(Model(b, name, blk + u32(b, e)))
    return out


# ---- display lists -> triangles --------------------------------------------------------------------

GX_PARAMS = {0x00: 0, 0x10: 1, 0x11: 0, 0x12: 1, 0x13: 1, 0x14: 1, 0x15: 0, 0x16: 16, 0x17: 12, 0x18: 16,
             0x19: 12, 0x1A: 9, 0x1B: 3, 0x1C: 3, 0x20: 1, 0x21: 1, 0x22: 1, 0x23: 2, 0x24: 1, 0x25: 1,
             0x26: 1, 0x27: 1, 0x28: 1, 0x29: 1, 0x2A: 1, 0x2B: 1, 0x30: 1, 0x31: 1, 0x32: 1, 0x33: 1,
             0x34: 32, 0x40: 1, 0x41: 0, 0x50: 1, 0x60: 1, 0x70: 3, 0x71: 2, 0x72: 1}


def decode_dl(b, at, size):
    """Display list bytes -> [(command, [words])]."""
    out = []
    o, end = at, at + size
    while o + 4 <= end:
        cmds = b[o:o + 4]
        o += 4
        for c in cmds:
            n = GX_PARAMS.get(c)
            if n is None:
                raise NsbmdError(f'GX command {c:#04x} at {o:#x}')
            if o + 4 * n > end:
                raise NsbmdError('display list cut off')
            out.append((c, [u32(b, o + 4 * i) for i in range(n)]))
            o += 4 * n
    return out


def sx(v, bits):
    v &= (1 << bits) - 1
    return v - (1 << bits) if v >> (bits - 1) else v


def triangles(b, at, size, pos_scale=1.0):
    """The triangles of a display list: [(restore index, [(x, y, z) x3])], positions in model units
    (the file's fixed point times `pos_scale`). Quad strips and quads are split into triangles. The
    restore index is the stack slot named by the last MTX_RESTORE (None before any)."""
    tris = []
    mode = None
    verts = []
    pos = (0.0, 0.0, 0.0)
    restore = None
    k = pos_scale / 4096

    def vertex(p):
        verts.append(p)
        n = len(verts)
        if mode == 0 and n % 3 == 0:
            tris.append((restore, verts[-3:]))
        elif mode == 1 and n % 4 == 0:
            q = verts[-4:]
            tris.append((restore, [q[0], q[1], q[2]]))
            tris.append((restore, [q[0], q[2], q[3]]))
        elif mode == 2 and n >= 3:
            a, c, d = verts[-3:]
            tris.append((restore, [a, c, d] if n % 2 else [c, a, d]))
        elif mode == 3 and n >= 4 and n % 2 == 0:
            a, c, d, e = verts[-4:]
            tris.append((restore, [a, c, d]))
            tris.append((restore, [d, c, e]))
    for c, w in decode_dl(b, at, size):
        if c == 0x14:
            restore = w[0] & 31
        elif c == 0x40:
            mode, verts = w[0] & 3, []
        elif c == 0x41:
            mode, verts = None, []
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
            # VTX_DIFF: the ten bits are added to the 4.12 place as they are
            pos = tuple(pos[i] + sx(w[0] >> (10 * i), 10) * k for i in range(3))
            vertex(pos)
    return tris


# ---- ROM -------------------------------------------------------------------------------------------

def rom_models(rom):
    """(path, bytes) of the ROM's `.nsbmd` files, decompressed."""
    for path, b in nitro.rom_files(rom):
        low = path.lower()
        if low.endswith('.nsbmd') or low.endswith('.nsbmd.z'):
            yield path, b


def main(argv):
    if not argv:
        print(__doc__)
        return 2
    if argv[0].endswith('.nds'):
        rom = ndsrom.Rom(open(argv[0], 'rb').read())
        if len(argv) > 1:
            for path, b in rom_models(rom):
                if path == argv[1] or path.endswith(argv[1]):
                    print(json.dumps([m.as_json() for m in parse(b)], indent=1))
                    return 0
            print('no such file:', argv[1], file=sys.stderr)
            return 1
        files = models = nodes = meshes = polys = 0
        bad = []
        for path, b in rom_models(rom):
            files += 1
            try:
                for m in parse(b):
                    models += 1
                    nodes += len(m.nodes)
                    meshes += len(m.shapes)
                    for s in m.shapes:
                        triangles(b, s['dl'][0], s['dl'][1])
                        polys += 1
            except (NsbmdError, struct.error, IndexError) as e:
                bad.append((path, e))
        print(f'{files} files, {models} models, {nodes} nodes, {meshes} meshes (display lists decoded: {polys})')
        for p, e in bad:
            print('  FAILED', p, e)
        return 1 if bad else 0
    b = open(argv[0], 'rb').read()
    if b[:4] != b'BMD0':
        b = bytes(lz.decompress(b))
    print(json.dumps([m.as_json() for m in parse(b)], indent=1))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
