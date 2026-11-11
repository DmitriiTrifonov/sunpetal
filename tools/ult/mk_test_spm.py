"""A synthetic SPM2 model for the swap test: a coloured cube and a fine grid, bound to a DS model by name.
usage: mk_test_spm.py OUT.spm DS_MODEL_NAME [GRID_N] [SIZE]"""
import struct
import sys

out, name = sys.argv[1], sys.argv[2]
grid = int(sys.argv[3]) if len(sys.argv) > 3 else 100
size = float(sys.argv[4]) if len(sys.argv) > 4 else 2.0

verts, idx = [], []


def v(p, c):
    verts.append((p, c))
    return len(verts) - 1


# a cube, each face its own colour
faces = [((1, 0, 0), (0, 1, 0), (0, 0, 1), (255, 0, 0)), ((-1, 0, 0), (0, 1, 0), (0, 0, 1), (0, 255, 0)), ((0, 1, 0), (1, 0, 0), (0, 0, 1), (0, 0, 255)),
         ((0, -1, 0), (1, 0, 0), (0, 0, 1), (255, 255, 0)), ((0, 0, 1), (1, 0, 0), (0, 1, 0), (0, 255, 255)), ((0, 0, -1), (1, 0, 0), (0, 1, 0), (255, 0, 255))]
h = size / 4
for n, a, b, col in faces:
    q = []
    for sa, sb in ((-1, -1), (1, -1), (1, 1), (-1, 1)):
        q.append(v(tuple(h * (n[k] + sa * a[k] + sb * b[k]) for k in range(3)), col))
    idx += [q[0], q[1], q[2], q[0], q[2], q[3]]
cube = len(idx)
# a grid in the plane y = -h: 2 * grid * grid triangles, a chequered grey
for i in range(grid):
    for j in range(grid):
        x0, x1 = -size + 2 * size * i / grid, -size + 2 * size * (i + 1) / grid
        z0, z1 = -size + 2 * size * j / grid, -size + 2 * size * (j + 1) / grid
        col = (200, 200, 200) if (i + j) % 2 else (60, 60, 60)
        q = [v((x0, -h, z0), col), v((x1, -h, z0), col), v((x1, -h, z1), col), v((x0, -h, z1), col)]
        idx += [q[0], q[1], q[2], q[0], q[2], q[3]]

o = b"SPM2" + bytes([1]) + struct.pack("<f", 1.0)
o += struct.pack("<H", len(name)) + name.encode() + struct.pack("<IH", 0, 0xFFFF)
o += struct.pack("<12f", 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0)
o += struct.pack("<H", 0)  # no bones
o += struct.pack("<H", 2) + bytes([0, 0, 0, 0]) + bytes([0, 1, 0, 0])  # two materials: pass 0 and pass 1, no picture
o += struct.pack("<I", len(verts))
for p, c in verts:
    o += struct.pack("<3f3f4B2fHH", *p, 0, 1, 0, c[0] // 2, c[1] // 2, c[2] // 2, 128, 0, 0, 0, 0)
o += struct.pack("<I", 2) + struct.pack("<HII", 1, 0, cube) + struct.pack("<HII", 0, cube, len(idx) - cube)
o += struct.pack("<I", len(idx)) + struct.pack("<%dI" % len(idx), *idx)
open(out, "wb").write(o)
print(len(verts), "vertices", len(idx) // 3, "triangles")
