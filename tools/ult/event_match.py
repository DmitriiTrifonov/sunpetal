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
# PS2 scene -> DS chapter by data/ult/scenario.toml (SIDtoRID: C08=R10, C09=R08, C10=R09; Y-59)
CHAP={s['ps2']:int(s['ds'][1:]) for s in tomllib.load(open(os.path.expanduser('~/Developer/games/sunpetal/data/ult/scenario.toml'),'rb'))['scenario'] if 'ds' in s}
def chap(scene): return CHAP.get(scene,-1)
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
good={k:[v[0]] for k,v in res.items() if v[1]<1.5}
# where several DS events stand at one point, sunpetal-ee Y-60 picks (out/y60/pick_shape.tsv: text, geometry of the .EVN offset);
# an event that branches by its visit flags is several DS scenes (text_multi: every scene of its branches, out/y60/branch.tsv)
Y60=EX+'/../../y60'
multi=collections.defaultdict(list)
for r in csv.DictReader(open(Y60+'/branch.tsv'),delimiter='\t'):
    multi[(r['ps2_scene'],int(r['event_id']))].append(r['ds_scene'])
picked=collections.Counter()
for r in csv.DictReader(open(Y60+'/pick_shape.tsv'),delimiter='\t'):
    k=(r['ps2_scene'],int(r['event_id'])); rule=r['final_rule']
    if rule=='text_multi' and k in multi: good[k]=sorted(set(multi[k]))
    elif r['final'] and not rule.startswith('unknown'): good[k]=[r['final']]
    else: continue
    picked[rule.split(':')[0]]+=1
print('Y-60 picks',dict(picked))
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

# the table the runtime reads: what a DS scene switches in a PS2 room (data/ult/zone_switch.tsv). A zone one event both turns on and off
# ends as the last switch by pc (the event runs through; branches are not followed, so this is a guess, column `rule` = last);
# a zone switched by several events of the scene with both states is left out (the order of the events is not known)
sw={}
for r in csv.DictReader(open(EX+'/../../y50/zone_events.tsv'),delimiter='\t'):
    k=(r['scene'],int(r['event']))
    if k in good:
        for ds_scene in good[k]:
            key=(ds_scene,r['scene'],r['scene3d'],r['kind'],r['zone'])
            sw.setdefault(key,[]).append((int(r['event']),int(r['pc']),r['onoff']))
rows=[]
for k,v in sorted(sw.items()):
    states={x[2] for x in v}
    if len(states)==1: rows.append((k,v[0][2],'only'))
    elif len({x[0] for x in v})==1: rows.append((k,max(v,key=lambda x:x[1])[2],'last'))
os.makedirs('data/ult',exist_ok=True)
open('data/ult/zone_switch.tsv','w').write('ds_scene\tps2_scene\tscene3d\tkind\tzone\tonoff\trule\n'+''.join('\t'.join(k)+'\t'+v+'\t'+rule+'\n' for k,v,rule in rows))
print('zone_switch rows',len(rows),'by last',sum(1 for r in rows if r[2]=='last'),'left out',len(sw)-len(rows))
