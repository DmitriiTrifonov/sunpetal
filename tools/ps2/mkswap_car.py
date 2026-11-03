"""mkswap5.py OUT rev vflip [flipfront]: подмена по ключу, E из orient (perm/signs/scale/offset)"""
import sys,struct,math
sys.path.insert(0,'tools/ps2')
import lf,g101_verts as g
out,rev,vflip=sys.argv[1],int(sys.argv[2]),int(sys.argv[3]); ff=int(sys.argv[4]) if len(sys.argv)>4 else 1
import os
perm=(0,2,1);sg=[-1,1,1];sc=float(os.environ.get('SC',0.04));off=[0.0002,-0.2,-0.6166]
if 'OFF' in os.environ: off=[float(x) for x in os.environ['OFF'].split(',')]
COL=os.environ.get('COL','90')
if ff: sg[0]=-sg[0]; sg[1]=-sg[1]   # развернуть спереди/назад (поворот на 180° вокруг вертикали DS)
a=lf.Archive(open('build/ps2-models/raw/OP.BIN','rb').read())
pt,_,_,_=g.entry_mesh(bytes(a.entries[18].data))
E=[[0]*3 for _ in range(3)]
for i in range(3): E[perm[i]][i]=sg[i]*sc
with open(out,'w') as f:
    f.write('key 128x128_5_8def2870adbfb901_f387cdfc24eb54ac\nsize 128\n'
            'hide 64x64_5_0430bc758dfac332_58a85f96628fe037\n')
    f.write('e '+' '.join('%.6f'%E[j][i] for j in range(3) for i in range(3))+' '+' '.join('%.6f'%x for x in off)+'\n')
    n=0
    for t in pt:
        if not all(math.isfinite(x) and abs(x)<1e4 for r in t for x in r[:3]): continue
        for k in ([0,2,1] if rev else [0,1,2]):
            r=t[k];u,v=r[12],r[13]
            if vflip: v=1-v
            f.write('v %.4f %.4f %.4f %.5f %.5f %s %s %s\n'%(r[0],r[1],r[2],u,v,COL,COL,COL)); 
        n+=1
print(n,'triangles')
