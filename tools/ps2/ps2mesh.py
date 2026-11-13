"""Меш записи архива PS2 через читатель на Rust (У1.13): замена g101_verts.entry_mesh и g101_nodes.place.

    import ps2mesh; tris, nlists, nstrips, bad = ps2mesh.entry_mesh(bytes_of_record)

Модели G101 читает `sunpetal-ps2-mesh` (crates/sunpetal-data/src/ps2/mesh.rs): вершины уже в позе покоя (кость
вершины, InitKey), грани развёрнуты по F-12. Треугольник — три вершины по 16 float: x y z 1, nx ny nz 1,
r g b 128, u v 1 0 (как у старого читателя). Бинарь собирается при первом вызове: cargo build --release.
"""
import os
import struct
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BIN = os.path.join(ROOT, 'target', 'release', 'sunpetal-ps2-mesh')


# ST за пределами ±64 — мусор в слоте (единицы вершин на модель): такие треугольники как у старого читателя не берём
UV_LIMIT = 64


def _binary():
    if not os.path.exists(BIN):
        subprocess.run(['cargo', 'build', '--release', '-p', 'sunpetal-data', '--bin', 'sunpetal-ps2-mesh'], cwd=ROOT, check=True)
    return BIN


def entry_mesh(d):
    """(треугольники, серий-списков, серий-полос, отброшено вырожденных) по байтам записи."""
    out = subprocess.run([_binary()], input=bytes(d), capture_output=True, check=True).stdout
    nl, ns, bad, nt = struct.unpack_from('<4I', out, 0)
    flat = struct.unpack_from('<%df' % (nt * 48), out, 16)
    tris = []
    for i in range(nt):
        o = i * 48
        t = (flat[o:o + 16], flat[o + 16:o + 32], flat[o + 32:o + 48])
        if all(abs(v[12]) <= UV_LIMIT and abs(v[13]) <= UV_LIMIT for v in t):
            tris.append(t)
        else:
            bad += 1
    return tris, nl, ns, bad
