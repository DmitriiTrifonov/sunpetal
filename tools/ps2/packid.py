#!/usr/bin/env python3
"""Id картинок 2D-пакета (K-092): манифест `pack.toml`, формат — `docs/2D-UPSCALE.md`, разбор — `sunpetal-data::pack2d`.

    packid.py fold DIR [IDS.csv]   файлы-по-ключам → {id}.png + pack.toml (одинаковые картинки сливаются в один id)
    packid.py index DUMP PACK      добавить в DUMP/index.csv колонку id по манифесту PACK (пусто — картинки нет в пакете)

IDS.csv: строки `id,ключ` (ключ — имя файла без .png); ключи без строки получают id `p_<8 hex sha1 содержимого>`.
Файлы по ключам остаются на месте только если `--keep`; манифест кладётся рядом, старые имена без него работают как раньше.
Ключ — имя выгрузки без `.png`: `obj0_16x32_{data}_{pal}`, `bg00_{hash}`, с `@группа.вариант`.
"""
import csv
import hashlib
import os
import re
import sys

KEY = re.compile(r'^(obj\d_\d+x\d+_[0-9a-f]{16}_[0-9a-f]{16}|bg\d\d_[0-9a-f]{16})(@[^.@]+\.[^.@]+)?$')


def read_manifest(text):
    """{id: [ключи]}; тот же поднабор TOML, что у Rust-разбора."""
    out, cur, open_ = {}, None, False
    for raw in text.splitlines():
        line = raw.split('#')[0].strip()
        if not line:
            continue
        if not open_ and line.startswith('['):
            cur = line.strip('[] ')
            out[cur] = []
            continue
        body = line if open_ else line.split('=', 1)[1].strip()
        body = body[1:] if not open_ else body
        done = ']' in body
        out[cur] += re.findall(r'"([^"]*)"', body.split(']')[0])
        open_ = not done
    return out


def write_manifest(ids):
    lines = ['# id → ключи (имена выгрузки без .png); {id}.png — картинка для всех ключей', '']
    for i in sorted(ids):
        lines.append('[%s]' % i)
        lines.append('keys = [%s]' % ', '.join('"%s"' % k for k in sorted(ids[i])))
        lines.append('')
    return '\n'.join(lines)


def fold(d, names=None, keep=False):
    names = names or {}
    by_hash, ids = {}, {}
    for f in sorted(os.listdir(d)):
        if not f.endswith('.png'):
            continue
        key = f[:-4]
        if not KEY.match(key):
            continue
        data = open(os.path.join(d, f), 'rb').read()
        h = hashlib.sha1(data).hexdigest()
        by_hash.setdefault(h, (data, []))[1].append(key)
    for h, (data, keys) in by_hash.items():
        named = [names[k] for k in keys if k in names]
        if len(set(named)) > 1:
            raise SystemExit('одна картинка под разными id: %s' % sorted(set(named)))
        i = named[0] if named else 'p_' + h[:8]
        ids.setdefault(i, []).extend(keys)
        open(os.path.join(d, i + '.png'), 'wb').write(data)
        if not keep:
            for k in keys:
                os.remove(os.path.join(d, k + '.png'))
    with open(os.path.join(d, 'pack.toml'), 'w') as f:
        f.write(write_manifest(ids))
    return ids


def add_index(dump, pack):
    """Колонка id в index.csv выгрузки (9 колонок рантайма + id); повторный вызов обновляет её."""
    ids = read_manifest(open(os.path.join(pack, 'pack.toml')).read())
    key_id = {k: i for i, ks in ids.items() for k in ks}
    p = os.path.join(dump, 'index.csv')
    rows = [r[:9] for r in csv.reader(open(p))]
    out = [rows[0] + ['id']]
    for r in rows[1:]:
        file = r[8]
        out.append(r + [key_id.get(file[:-4] if file.endswith('.png') else file, '')])
    with open(p, 'w', newline='') as f:
        csv.writer(f, lineterminator='\n').writerows(out)
    return sum(1 for r in out[1:] if r[9])


def main(a):
    if len(a) >= 2 and a[0] == 'fold':
        names = {}
        if len(a) > 2:
            for r in csv.reader(open(a[2])):
                if len(r) >= 2 and not r[0].startswith('#'):
                    names[r[1]] = r[0]
        ids = fold(a[1], names)
        print('id: %d, ключей: %d' % (len(ids), sum(len(v) for v in ids.values())))
    elif len(a) == 3 and a[0] == 'index':
        print('строк с id:', add_index(a[1], a[2]))
    else:
        print(__doc__)
        return 2
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
