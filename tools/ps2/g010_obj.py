#!/usr/bin/env python3
"""G010 PS2 -> OBJ (K-153): блоки Prim=5, три вершины по 0x40 Б. g010_obj.py G010.bin OUT.obj (запись выгрузить из LF)."""
import re,struct,sys
f=sys.argv[1]; r=open(f,'rb').read()
ps=[m.start() for m in re.finditer(rb'Prim\x00\x00\x00\x00',r)]
V=[];T=[];N=[];F=[]
for a in ps:
    if struct.unpack_from('<I',r,a+8)[0]!=5: continue
    base=len(V)
    for k in range(3):
        o=a+0x28+k*0x40
        x,y,z,_=struct.unpack_from('<4f',r,o); nx,ny,nz,_=struct.unpack_from('<4f',r,o+16); u,v,_,_=struct.unpack_from('<4f',r,o+48)
        V.append((x,y,z));N.append((nx,ny,nz));T.append((u,v))
    F.append((base,base+1,base+2))
print(len(F),'tris', [ (min(c[i] for c in V),max(c[i] for c in V)) for i in range(3)])
print('uv',min(t[0] for t in T),max(t[0] for t in T),min(t[1] for t in T),max(t[1] for t in T))
with open(sys.argv[2],'w') as w:
    for v in V:w.write('v %f %f %f\n'%v)
    for t in T:w.write('vt %f %f\n'%t)
    for n in N:w.write('vn %f %f %f\n'%n)
    for a,b,c in F:w.write(f'f {a+1}/{a+1}/{a+1} {b+1}/{b+1}/{b+1} {c+1}/{c+1}/{c+1}\n')
