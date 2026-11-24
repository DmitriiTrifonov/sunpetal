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

# the table the runtime reads: what a DS scene switches in a PS2 room (data/ult/zone_switch.tsv). The value of a zone per event is
# sunpetal-ee Y-56b (out/y56b/zone_branch.tsv, paths of the event followed): `fixed` every path that touches the zone gives one value,
# `branch` it hangs on var 0x6002[0]/flags at the event's start (see below), an unreached zone is left out; so is a zone switched by several events of the scene with both states (the order is not known)
ZB={}
for r in csv.DictReader(open(EX+'/../../y56b/zone_branch.tsv'),delimiter='\t'):
    ZB[(r['scene'],int(r['event']),r['scene3d'],r['kind'],r['zone'])]=(r['rule'],r['final'])
sw={}
for r in csv.DictReader(open(EX+'/../../y50/zone_events.tsv'),delimiter='\t'):
    k=(r['scene'],int(r['event']))
    if k in good:
        for ds_scene in good[k]:
            key=(ds_scene,r['scene'],r['scene3d'],r['kind'],r['zone'])
            sw.setdefault(key,[]).append((int(r['event']),int(r['pc']),r['onoff']))
rows=[];why=collections.Counter()
for k,v in sorted(sw.items()):
    per={}
    for e in sorted({x[0] for x in v}):
        rule,final=ZB.get((k[1],e,k[2],k[3],k[4]),('?',''))
        if rule=='fixed': per[e]=('fixed','on' if final=='1' else 'off')
        elif rule=='branch': per[e]=('branch',max((x for x in v if x[0]==e),key=lambda x:x[1])[2])
        else: per[e]=None
    if any(p is None for p in per.values()): why['unreached or missing']+=1; continue
    if len({p[1] for p in per.values()})>1: why['several events, both states']+=1; continue
    rows.append((k,next(iter(per.values()))[1],'branch' if any(p[0]=='branch' for p in per.values()) else 'fixed'))
# the `branch` zones are the hotel stairs (and R11's spiral): var 0x6002[0] is the floor, the switches go to the stairs' room only
# (sunpetal-ee Y-56c/Y-56d, out/y56d/verdict_room.tsv, live in R03 32/32): `resolved` takes the floor's value (rule `stairs`);
# `false_map` (a stair event bound by place to a story scene) and `not_touched` (the stairs do not write that room) are left out
VR={}
for r in csv.DictReader(open(EX+'/../../y56d/verdict_room.tsv'),delimiter='\t'):
    VR[(r['ds_scene'],r['ps2_scene'],r['scene3d'],r['kind'],r['zone'])]=(r['verdict'],r['final'])
kept=[]
for k,v,rule in rows:
    if rule=='branch':
        verdict,final=VR.get(k,('?',''))
        if verdict!='resolved': why[verdict]+=1; continue
        v,rule=('on' if final=='1' else 'off'),'stairs'
    kept.append((k,v,rule))
rows=kept
os.makedirs('data/ult',exist_ok=True)
open('data/ult/zone_switch.tsv','w').write('ds_scene\tps2_scene\tscene3d\tkind\tzone\tonoff\trule\n'+''.join('\t'.join(k)+'\t'+v+'\t'+rule+'\n' for k,v,rule in rows))
print('zone_switch rows',len(rows),dict(collections.Counter(r[2] for r in rows)),'left out',dict(why))

# the zones a PS2 event switches when the hero walks into its automatic event zone (data/ult/zone_trigger.tsv, nativewalk::frame):
# only the events checked live — R11 #39/#40, the lighthouse switch of room 5 (sunpetal-ee F-272: event zones 3,4 -> #39, 5 -> #40,
# the state is the last zone walked through, 20/20 live). Walls and floors are Y-56b `fixed` rows of that room; event zones are
# zones.tsv on/off lists (an event in both is left out); camera zones are not followed by the runtime
TRIGGERS={('R11',5):(39,40)}
trig=[]
for r in csv.DictReader(open(EX+'/../../y56b/zone_branch.tsv'),delimiter='\t'):
    k=(r['scene'],int(r['scene3d']))
    if int(r['event']) in TRIGGERS.get(k,()) and r['rule']=='fixed':
        trig.append((r['scene'],r['scene3d'],r['event'],r['kind'],r['zone'],'on' if r['final']=='1' else 'off'))
for r in csv.DictReader(open(EX+'/zones.tsv'),delimiter='\t'):
    k=(r['scene'],int(r['scene3d']))
    if r['kind']!='event' or k not in TRIGGERS: continue
    evs=lambda col:{int(x.split('@')[0]) for x in r[col].split(',') if x}
    for e in TRIGGERS[k]:
        on,off=e in evs('on_events'),e in evs('off_events')
        if on!=off: trig.append((r['scene'],r['scene3d'],str(e),'event',r['zone'],'on' if on else 'off'))
trig=sorted(set(trig),key=lambda t:(t[0],int(t[1]),int(t[2]),t[3],int(t[4])))
open('data/ult/zone_trigger.tsv','w').write('ps2_scene\tscene3d\tevent\tkind\tzone\tonoff\n'+''.join('\t'.join(t)+'\n' for t in trig))
print('zone_trigger rows',len(trig),dict(collections.Counter((t[2],t[3],t[5]) for t in trig)))
