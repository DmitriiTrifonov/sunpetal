"""The zones of a stairs' room on each floor (data/ult/stairs_floor.tsv, read by nativewalk::stairs_zones).
R03, R04: sunpetal-ee Y-56e (out/y56e/predict.tsv, live 166/166 and 212/212, all four kinds, the flight by the event's own
instance x); other scenes: Y-56d (out/y56d/stairs_floor_room.tsv, hit/height, the flight by the scene's union of instances).
A zone two events leave differently on the same floor of the same flight is left out (none at the time of writing)."""
import csv,os,collections
Y=os.path.expanduser('~/Developer/games/sunpetal-ee/out')
LIVE={'R03','R04'}
got=collections.defaultdict(set)
for r in csv.DictReader(open(Y+'/y56e/predict.tsv'),delimiter='\t'):
    got[(r['ps2_scene'],int(r['scene3d']),r['kind'],int(r['zone']),int(r['after']),'y56e')].add(r['final'])
for r in csv.DictReader(open(Y+'/y56d/stairs_floor_room.tsv'),delimiter='\t'):
    if r['ps2_scene'] not in LIVE:
        got[(r['ps2_scene'],int(r['scene3d']),r['kind'],int(r['zone']),int(r['floor']),'y56d')].add(r['final'])
rows=sorted((k,v.pop()) for k,v in got.items() if len(v)==1)
both=sum(1 for v in got.values() if len(v)>1)
open('data/ult/stairs_floor.tsv','w').write('ps2_scene\tscene3d\tkind\tzone\tfloor\tfinal\tsource\n'+''.join(
    f'{k[0]}\t{k[1]}\t{k[2]}\t{k[3]}\t{k[4]}\t{v}\t{k[5]}\n' for k,v in rows))
print('rows',len(rows),dict(collections.Counter((k[5],k[2]) for k,_ in rows)),'left out (both states)',both)
