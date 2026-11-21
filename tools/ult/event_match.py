"""PS2 event area -> DS event by place (Y-50/Y-51 from our side): the area centre in PS2 scene units goes to DS units by the
area's `shift` (data/ult/areas.toml), and the nearest DS .EVN event of the same map and chapter within 1.5 is its scene.
Writes build/zones/event_map.tsv. Reads fsar-ru/build/data (read only) and the sunpetal-ee export."""
import struct,glob,json,csv,os,re,collections
EX=os.path.expanduser('~/Developer/games/sunpetal-ee/out/export/v1')
import tomllib
SHA={}
for a in tomllib.load(open(os.path.expanduser('~/Developer/games/sunpetal/data/ult/areas.toml'),'rb'))['area']:
    m=re.search(r'/A(\d+)/',a['file'])
    if m and 'shift' in a: SHA[int(m.group(1))]=a['shift']
def evn(path):
    d=open(path,'rb').read(); out=[]; at=0
    while at+104<=len(d):
        nm=d[at:at+24].split(b'\0')[0]
        if not re.fullmatch(rb'[A-Z]\d\d_\w+\.TXT',nm): at+=4; continue
        w=struct.unpack_from('<26i',d,at)
        out.append(dict(scene=nm[:-4].decode(),map=w[15],pos=(w[8]/4096,w[9]/4096,w[10]/4096)))
        at+=104
    return out
ds=[]
for p in glob.glob(os.path.expanduser('~/Developer/games/fsar-ru/build/data/Chapter/Pack/*/*.EVN')): ds+=evn(p)
bymap=collections.defaultdict(list)
for v in ds: bymap[v['map']].append(v)
rooms=collections.defaultdict(set) # (scene,idx)->maps
for r in csv.DictReader(open(EX+'/maps.tsv'),delimiter='\t'):
    m=int(r['ds_area'][1:])
    for x in r['ps2_rooms'].split(';'):
        if '#' in x:
            s,i=x.split('#'); rooms[(s,int(i))].add(m)
def chap(scene): # PS2 scene -> DS chapter
    if scene=='OP': return 0
    n=int(scene[1:]); return n if n<=13 else n-1
res={}  # (scene,event_id)->(ds_scene,dist)
dist_all=[]
for f in sorted(glob.glob(EX+'/scenes/*.json')):
    sc=os.path.basename(f)[:-5]; d=json.load(open(f))
    for idx,s3 in enumerate(d['scene3d']):
        i=s3.get('index',idx)
        maps=rooms.get((sc,i),set())
        cand=[(v,m) for m in maps if m in SHA for v in bymap[m] if int(v['scene'][1:3])==chap(sc)]
        for e in s3.get('event',[]):
            if e.get('event_id',-1)<0 or not cand: continue
            if e['shape']==3: cx,cz=e['x0']+e['dx']/2,e['z0']+e['dz']/2
            elif e['shape']==2: cx,cz=e['cx'],e['cz']
            else: continue
            def dd(c):
                v,m=c; sh=SHA[m]; dx,dz=cx/10+sh[0],cz/10+sh[2]
                return ((v['pos'][0]-dx)**2+(v['pos'][2]-dz)**2)**.5
            bc=min(cand,key=dd); best=bc[0]; dist=dd(bc)
            dist_all.append(dist)
            res[(sc,e['event_id'])]=(best['scene'],dist)
os.makedirs('build/zones',exist_ok=True)
open('build/zones/event_map.tsv','w').write('scene\tevent\tds_scene\tdist\n'+''.join(f'{k[0]}\t{k[1]}\t{v[0]}\t{v[1]:.2f}\n' for k,v in sorted(res.items())))
good={k:v for k,v in res.items() if v[1]<1.5}
print('areas with candidates',len(res),'dist<1.5:',len(good))
import statistics
print('quantiles',[round(sorted(dist_all)[int(len(dist_all)*q)],2) for q in (.1,.25,.5,.75,.9)])
rows=0;cov=0;zs=set();czs=set()
for r in csv.DictReader(open(EX+'/../../y50/zone_events.tsv'),delimiter='\t'):
    rows+=1; k=(r['scene'],int(r['event']))
    zk=(r['scene'],r['scene3d'],r['kind'],r['zone']); zs.add(zk)
    if k in good: cov+=1; czs.add(zk)
print('rows',rows,'covered',cov,'zones',len(zs),'covered zones',len(czs))

old=set()
for r in csv.DictReader(open(EX+'/../../y50/zone_events.tsv'),delimiter='\t'):
    if r['confidence']!='unknown': old.add((r['scene'],r['scene3d'],r['kind'],r['zone']))
print('zones new over the text anchors:',len(czs-old),'of',len(czs))
