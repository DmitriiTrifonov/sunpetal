#!/usr/bin/env python3
"""Приёмка выгрузки `sunpetal-ee` (`out/export/v1`) против контракта (docs/ULTIMATE-EE-TASKS.md, раздел 2; У0.2).

    check_export.py DIR [--fast] [--expect-scenes 20]

Ошибки (код возврата 1): схема манифеста; файл из манифеста отсутствует, размер или SHA-1 не те (`--fast`
пропускает SHA-1); счётчики манифеста не равны числу файлов; паспорт модели не сходится с `.glb`/`.vtx.bin`
(размер = 40 байт на вершину, серии в пределах вершин, материалы в пределах списка); картинка материала или
`textures.tsv` не найдена в `textures/`; сцена ссылается на несуществующую модель или движение; строка
`events/*.jsonl` не разбирается. Предупреждения: файлы на диске, которых нет в манифесте (выгрузка новее манифеста).
Только читает.
"""
import hashlib
import json
import os
import sys

VTX_STRIDE = 40


def sha1_of(path):
    h = hashlib.sha1()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def check(root, fast=False, expect_scenes=20):
    """Возвращает (ошибки, предупреждения, сводка)."""
    errs, warns, info = [], [], {}
    mpath = os.path.join(root, 'manifest.json')
    if not os.path.isfile(mpath):
        return ['manifest.json: нет файла'], warns, info
    man = json.load(open(mpath))
    if man.get('schema') != 1:
        errs.append(f'manifest: schema = {man.get("schema")!r}, ожидалась 1')
    listed = set()
    for ent in man.get('files', []):
        p = ent['path']
        listed.add(p)
        full = os.path.join(root, p)
        if not os.path.isfile(full):
            errs.append(f'{p}: из манифеста, но файла нет')
            continue
        if os.path.getsize(full) != ent['size']:
            errs.append(f'{p}: размер {os.path.getsize(full)} != {ent["size"]}')
        elif not fast and sha1_of(full) != ent['sha1']:
            errs.append(f'{p}: SHA-1 не совпадает с манифестом')
    extra = 0
    for d, _, fs in os.walk(root):
        for f in fs:
            rel = os.path.relpath(os.path.join(d, f), root).replace(os.sep, '/')
            if rel != 'manifest.json' and rel not in listed:
                extra += 1
    if extra:
        warns.append(f'{extra} файлов на диске нет в манифесте (манифест от {man.get("date")} старше выгрузки?)')

    def ls(sub, ext):
        d = os.path.join(root, sub)
        return sorted(f[:-len(ext)] for f in os.listdir(d) if f.endswith(ext)) if os.path.isdir(d) else []

    models, motions, scenes = ls('models', '.json'), ls('motions', '.json'), ls('scenes', '.json')
    pngs = set(ls('textures', '.png'))
    info.update(models=len(models), motions=len(motions), scenes=len(scenes), textures=len(pngs))
    cnt = man.get('counts', {})
    want_models = cnt.get('models_in_scenes', 0) + cnt.get('models_extra', 0)
    if want_models and want_models != len(models):
        errs.append(f'моделей {len(models)}, в манифесте {want_models}')
    if cnt.get('motions') is not None and cnt['motions'] != len(motions):
        errs.append(f'движений {len(motions)}, в манифесте {cnt["motions"]}')
    if expect_scenes and len(scenes) != expect_scenes:
        errs.append(f'сцен {len(scenes)}, ожидалось {expect_scenes}')

    tsv = os.path.join(root, 'textures.tsv')
    rows = 0
    if os.path.isfile(tsv):
        for i, line in enumerate(open(tsv, encoding='utf-8')):
            if i == 0:
                continue
            rows += 1
            sha = line.split('\t', 1)[0]
            if sha not in pngs:
                errs.append(f'textures.tsv: {sha} без textures/{sha}.png')
    else:
        errs.append('textures.tsv: нет файла')
    if rows != len(pngs):
        warns.append(f'textures.tsv {rows} строк, картинок {len(pngs)}')

    model_set, motion_set = set(models), set(motions)
    for mid in models:
        try:
            m = json.load(open(os.path.join(root, 'models', mid + '.json')))
        except ValueError as e:
            errs.append(f'models/{mid}.json: {e}')
            continue
        if m.get('id') != mid or not str(m.get('sha1', '')).startswith(mid):
            errs.append(f'models/{mid}.json: id/sha1 не сходятся с именем')
        if not os.path.isfile(os.path.join(root, 'models', mid + '.glb')):
            errs.append(f'models/{mid}.glb: нет файла')
        nv = m.get('counts', {}).get('vertices', 0)
        vp = os.path.join(root, 'models', mid + '.vtx.bin')
        if not os.path.isfile(vp):
            errs.append(f'models/{mid}.vtx.bin: нет файла')
        elif os.path.getsize(vp) != nv * VTX_STRIDE:
            errs.append(f'models/{mid}.vtx.bin: {os.path.getsize(vp)} байт, ждали {nv}×{VTX_STRIDE}')
        mats = m.get('materials', [])
        for i, mat in enumerate(mats):
            t = mat.get('texture')
            if t is not None and t not in pngs:
                errs.append(f'models/{mid}.json: материал {i}: картинки {t} нет')
        top = 0
        for j, s in enumerate(m.get('series', [])):
            if not 0 <= s['material'] < len(mats):
                errs.append(f'models/{mid}.json: серия {j}: материал {s["material"]} вне списка')
            top = max(top, s['first_vertex'] + s['count'])
        if top > nv:
            errs.append(f'models/{mid}.json: серии доходят до вершины {top}, всего {nv}')

    for sid in scenes:
        try:
            s = json.load(open(os.path.join(root, 'scenes', sid + '.json')))
        except ValueError as e:
            errs.append(f'scenes/{sid}.json: {e}')
            continue
        refs = [v for r in s.get('scene3d', []) for v in r.get('models', {}).values()]
        refs += [v for c in s.get('chara', []) for v in c.get('models', {}).values()]
        for v in refs:
            if v and v not in model_set:
                errs.append(f'scenes/{sid}.json: модели {v} нет')
        for p in s.get('motion_pack', []):
            if p.get('motion') and p['motion'] not in motion_set:
                errs.append(f'scenes/{sid}.json: движения {p["motion"]} нет')
    for sid in ls('events', '.jsonl'):
        for n, line in enumerate(open(os.path.join(root, 'events', sid + '.jsonl'), encoding='utf-8'), 1):
            try:
                json.loads(line)
            except ValueError:
                errs.append(f'events/{sid}.jsonl:{n}: не JSON')
                break
    return errs, warns, info


def main(argv):
    args = [a for a in argv if not a.startswith('--')]
    fast = '--fast' in argv
    scenes = 20
    if '--expect-scenes' in argv:
        scenes = int(argv[argv.index('--expect-scenes') + 1])
        args = [a for a in args if a != str(scenes)]
    if len(args) != 1:
        print(__doc__)
        return 2
    errs, warns, info = check(args[0], fast, scenes)
    print('сводка:', ', '.join(f'{k} {v}' for k, v in info.items()))
    for w in warns:
        print('предупреждение:', w)
    for e in errs[:200]:
        print('ОШИБКА:', e)
    if len(errs) > 200:
        print(f'… и ещё {len(errs) - 200}')
    print(f'ошибок {len(errs)}, предупреждений {len(warns)}')
    return 1 if errs else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
