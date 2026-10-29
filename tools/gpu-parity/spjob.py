#!/usr/bin/env python3
"""Reads a .spjob job dump (crates/sunpetal-runtime/src/nds/jobdump.rs) and reports on it.
The format: MAGIC(4) scale(4) vblank(8) notes(4) npx(4) px(4*npx) nverts(4) vert(48*nverts)
npolys(4) poly(25*npolys) wbuffer(1) disp3dcnt(2) width(4) clear(4) cdepth(2) clrim(2) aref(1)
extras(4f+2b) edge(16) fog_color(4) fog_offset(2) fog_table(32) toon(64) tex(N) texpal(N)
keys(4 + 21*nkeys) pack_active(1)."""
import struct, sys, glob, os

def read(path):
    d = open(path, 'rb').read()
    assert d[:4] == b'SPJ1', path
    o = 4
    scale = struct.unpack_from('<I', d, o)[0]; o += 4
    vblank = struct.unpack_from('<Q', d, o)[0]; o += 8
    notes = struct.unpack_from('<I', d, o)[0]; o += 4
    npx = struct.unpack_from('<I', d, o)[0]; o += 4
    o += 4 * npx
    nverts = struct.unpack_from('<I', d, o)[0]; o += 4
    verts = [struct.unpack_from('<4I', d, o + 48*i) for i in range(nverts)]
    o += 48 * nverts
    npolys = struct.unpack_from('<I', d, o)[0]; o += 4
    polys = []
    for i in range(npolys):
        first, n, attr, tex, pal = struct.unpack_from('<5I', d, o)
        translucent = d[o + 20]
        polys.append(dict(first=first, n=n, attr=attr, tex=tex, pal=pal, translucent=translucent))
        o += 25
    wbuffer = d[o]; o += 1
    disp3dcnt = struct.unpack_from('<H', d, o)[0]; o += 2
    width = struct.unpack_from('<I', d, o)[0]; o += 4
    clear = struct.unpack_from('<I', d, o)[0]; o += 4
    cdepth = struct.unpack_from('<H', d, o)[0]; o += 2
    clrim = struct.unpack_from('<H', d, o)[0]; o += 2
    aref = d[o]; o += 1
    outline, ow, shadow, sr = struct.unpack_from('<4f', d, o); o += 16
    bil = d[o]; msaa = d[o+1]; o += 2
    return dict(path=path, scale=scale, vblank=vblank, notes=notes, px=npx, verts=verts, polys=polys,
                wbuffer=wbuffer, disp3dcnt=disp3dcnt, width=width, clear=clear, cdepth=cdepth,
                clrim=clrim, aref=aref, outline=outline, shadow=shadow, msaa=msaa, size=len(d))

def screen_x(v):
    return v[0]/16 + 128

def stripes(r, lo=208, hi=256):
    out = []
    for i, p in enumerate(r['polys']):
        if not p['translucent'] or p['n'] < 3:
            continue
        xs = [screen_x(r['verts'][p['first']+j]) for j in range(p['n'])]
        if min(xs) >= lo and max(xs) <= hi:
            out.append((i, round(min(xs)), round(max(xs))))
    return out

def summary(r):
    return (f"{os.path.basename(r['path']):34s} v{r['vblank']} scale {r['scale']} verts {len(r['verts']):5d} "
            f"polys {len(r['polys']):4d} trans {sum(1 for p in r['polys'] if p['translucent']):3d} "
            f"wbuf {r['wbuffer']} disp {r['disp3dcnt']:04X} width {r['width']} "
            f"extras(outline {r['outline']} shadow {r['shadow']} msaa {r['msaa']}) notes {r['notes']}")

if __name__ == '__main__':
    for p in sys.argv[1:]:
        r = read(p)
        print(summary(r))
        if '--stripes' in sys.argv:
            print('   stripes:', stripes(r))
