#!/usr/bin/env python3
"""Матрицы частей моделей PS2 (K-184): таблица узлов записи и расстановка частей по узлам. Только чтение.

    g101_nodes.py ARCHIVE.BIN [ЗАПИСЬ]    узлы записи и результат расстановки (какие части куда)

Факт (разведка 2026-11-09, OP#18, docs/PS2-RECON.md): в записи лежит таблица узлов с шагом 0x140, как у скелета
`G010`, но со смещением 0x70 (первый узел на 0x170). В узле три единичные матрицы 4×4 (место рантайма; в дампе ОЗУ
игры они заполнены позой кадра), на `+0x100` слова `child+1, sibling+1`, id (`0x77e4….`) и шесть `f32`: смещение
xyz относительно родителя и три малых числа (по виду углы в радианах). У `OP#18` узлов восемь: корень (кузов),
узел 1 с наклоном 20° (руль), узлы 2 и 3 (двери, x ±7,86), узлы 4–7 (колёса, x ±6,34, z +12,12 и −9,94).

Вершины частей лежат в локальных координатах (колесо — вокруг нуля), а **явной ссылки «серия вершин → узел» в записи
не найдено**: id узлов встречаются только в таблице. Поэтому часть → узел выводится по геометрии:
  1. «локальные» части — связные компоненты (кроме главной), чей bbox содержит начало координат (с допуском) и не
     больше 0,4 диагонали модели;
  2. узлы и части сводятся в зеркальные пары (узлы: равные y, z, противоположные x; части: равное число треугольников и
     противоположный знак центра по x) и одиночки; одиночки с одиночками, пары с парами;
  3. пара частей ↔ пара узлов и ориентация внутри пары выбираются перебором по стоимости: часть, поставленная в узел,
     не должна торчать из bbox главной части и уходить под землю (y < 0); при равной стоимости (передние и задние колёса,
     ступица наружу) — по порядку в записи. Дверь тянется внутрь, колесо ступицей наружу: знак по паре разный.
Если число локальных частей не равно числу узлов со смещением, запись остаётся как есть (`reason` в отчёте).
"""
import itertools
import math
import os
import struct
import sys

NODE = 0x140
MAX_NODES = 64


def _ident(d, o):
    if o < 0 or o + 64 > len(d):
        return False
    f = struct.unpack_from('<16f', d, o)
    return f == (1.0, 0, 0, 0, 0, 1.0, 0, 0, 0, 0, 1.0, 0, 0, 0, 0, 1.0)


def read_nodes(d):
    """Узлы записи: список {'i', 'child', 'sib', 'id', 't': (x, y, z), 'rot': (rx, ry, rz)}; пусто, если таблицы нет."""
    first = None
    for o in range(0, min(len(d) - 0x100, 0x2000), 16):
        if _ident(d, o) and _ident(d, o + 0x40) and _ident(d, o + 0x80):
            first = o - 0x40
            break
    if first is None or first < 0:
        return []
    nodes = []
    for i in range(MAX_NODES):
        b = first + NODE * i
        if not _ident(d, b + 0x40):
            break
        ch, sib = struct.unpack_from('<HH', d, b + 0x100 + 4)
        nid, = struct.unpack_from('<I', d, b + 0x100 + 12)
        v = struct.unpack_from('<6f', d, b + 0x100 + 16)
        if not all(math.isfinite(x) and abs(x) < 1e4 for x in v):
            break
        nodes.append({'i': i, 'child': ch - 1 if ch != 0xffff else None, 'sib': sib - 1 if sib != 0xffff else None,
                      'id': nid, 't': tuple(v[:3]), 'rot': tuple(v[3:])})
    return nodes


def world_offsets(nodes):
    """Смещение каждого узла от начала записи: сумма смещений по цепочке родителей (дети — через child и sibling)."""
    parent = {}
    for n in nodes:
        c = n['child']
        while c is not None and c < len(nodes) and c not in parent:
            parent[c] = n['i']
            c = nodes[c]['sib']
    out = []
    for n in nodes:
        t, p, seen = list(n['t']), parent.get(n['i']), set()
        while p is not None and p not in seen:
            seen.add(p)
            t = [a + b for a, b in zip(t, nodes[p]['t'])]
            p = parent.get(p)
        out.append(tuple(t))
    return out


def _pos(v):
    return v[0:3]


def comp_lists(tris, diag):
    """Связные компоненты треугольников по общим вершинам (допуск 1e-4 диагонали): списки индексов, по порядку появления."""
    q = max(diag * 1e-4, 1e-9)
    key, parent = {}, []

    def find(x):
        while parent[x] != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x

    def vid(p):
        k = (round(p[0] / q), round(p[1] / q), round(p[2] / q))
        if k not in key:
            key[k] = len(parent)
            parent.append(len(parent))
        return key[k]

    ids = [[vid(_pos(v)) for v in t] for t in tris]
    for a, b, c in ids:
        ra, rb = find(a), find(b)
        parent[rb] = ra
        parent[find(c)] = find(a)
    comps = {}
    for n, t in enumerate(ids):
        comps.setdefault(find(t[0]), []).append(n)
    return sorted(comps.values(), key=lambda c: c[0])


def _bbox(tris, idx):
    pts = [_pos(v) for n in idx for v in tris[n]]
    return [(min(p[a] for p in pts), max(p[a] for p in pts)) for a in range(3)]


def _rot(rot):
    """Матрица поворота Rz·Ry·Rx из трёх углов (радианы)."""
    rx, ry, rz = rot
    cx, sx, cy, sy, cz, sz = math.cos(rx), math.sin(rx), math.cos(ry), math.sin(ry), math.cos(rz), math.sin(rz)
    return [[cz * cy, cz * sy * sx - sz * cx, cz * sy * cx + sz * sx],
            [sz * cy, sz * sy * sx + cz * cx, sz * sy * cx - cz * sx],
            [-sy, cy * sx, cy * cx]]


def _apply(v, r, t):
    p = [sum(r[i][j] * v[j] for j in range(3)) + t[i] for i in range(3)]
    n = [sum(r[i][j] * v[4 + j] for j in range(3)) for i in range(3)]
    return tuple(p) + (v[3],) + tuple(n) + tuple(v[7:])


def _cost(bb, t, body, margin):
    """Насколько часть с bbox bb, поставленная в смещение t, торчит из главной части и уходит под землю."""
    c = 0.0
    for a in range(3):
        lo, hi = bb[a][0] + t[a], bb[a][1] + t[a]
        c += max(0.0, body[a][0] - margin - lo) + max(0.0, hi - (body[a][1] + margin))
    c += max(0.0, -(bb[1][0] + t[1]))
    return c


def _groups(items, same):
    """Зеркальные пары (по предикату same(a, b)) и одиночки: ([(a, b)], [a])."""
    left, pairs, singles = list(items), [], []
    while left:
        a = left.pop(0)
        for k, b in enumerate(left):
            if same(a, b):
                pairs.append((a, left.pop(k)))
                break
        else:
            singles.append(a)
    return pairs, singles


def place(tris, nodes):
    """Ставит локальные части по узлам. Возвращает (треугольники, отчёт). Без таблицы узлов или при несходящемся числе — как есть."""
    rep = {'nodes': len(nodes), 'local': 0, 'placed': 0, 'reason': ''}
    pts = [_pos(v) for t in tris for v in t]
    if not nodes or not pts:
        rep['reason'] = 'no nodes' if not nodes else 'empty'
        return tris, rep
    wo = world_offsets(nodes)
    moved = [k for k, n in enumerate(nodes) if max(abs(x) for x in wo[k]) > 1e-6 or max(abs(x) for x in n['rot']) > 1e-6]
    bb = [(min(p[a] for p in pts), max(p[a] for p in pts)) for a in range(3)]
    diag = math.dist([b[0] for b in bb], [b[1] for b in bb])
    comps = comp_lists(tris, diag)
    body = max(comps, key=len)
    bbody = _bbox(tris, body)
    tol = 0.03 * diag
    local = []
    for c in comps:
        if c is body:
            continue
        b = _bbox(tris, c)
        small = max(b[a][1] - b[a][0] for a in range(3)) <= 0.4 * diag  # оболочка размером с кузов — не часть
        if small and all(b[a][0] <= tol and b[a][1] >= -tol for a in range(3)):
            local.append({'idx': c, 'n': len(c), 'bb': b, 'cx': (b[0][0] + b[0][1]) / 2})
    rep['local'] = len(local)
    if not local or len(local) != len(moved):
        rep['reason'] = 'local parts %d, nodes with offsets %d' % (len(local), len(moved))
        return tris, rep
    nd = [{'k': k, 't': wo[k], 'rot': nodes[k]['rot']} for k in moved]

    def npair(a, b):
        return abs(a['t'][1] - b['t'][1]) < 1e-3 and abs(a['t'][2] - b['t'][2]) < 1e-3 and abs(a['t'][0] + b['t'][0]) < 1e-3 and abs(a['t'][0]) > 1e-3

    def cpair(a, b):
        return a['n'] == b['n'] and a['cx'] * b['cx'] < 0 and abs(a['cx'] + b['cx']) < 0.1 * max(abs(a['cx']), abs(b['cx']), 1e-9) * 2

    np_, ns = _groups(nd, npair)
    cp, cs = _groups(local, cpair)
    if len(np_) != len(cp) or len(ns) != len(cs):
        rep['reason'] = 'pairs: nodes %d+%d, parts %d+%d' % (len(np_), len(ns), len(cp), len(cs))
        return tris, rep
    margin = 0.01 * diag

    def pair_cost(cg, ng):
        """Лучшая ориентация внутри зеркальной пары: часть с меньшим x к узлу с меньшим x (0) или наоборот (1)."""
        cs_ = sorted(cg, key=lambda c: c['cx'])
        ns_ = sorted(ng, key=lambda n: n['t'][0])
        opts = []
        for o, order in enumerate((cs_, cs_[::-1])):
            opts.append((sum(_cost(c['bb'], n['t'], bbody, margin) for c, n in zip(order, ns_)), o, list(zip(order, ns_))))
        return min(opts, key=lambda t: (round(t[0], 6), t[1]))  # при равной стоимости — как есть (ступица колеса наружу)

    best = None
    for perm in itertools.permutations(range(len(cp))):
        for sperm in itertools.permutations(range(len(cs))):
            cost, chosen = 0.0, []
            for gi, pi in enumerate(perm):
                c_, _, pr = pair_cost(cp[pi], np_[gi])
                cost += c_
                chosen += pr
            for gi, pi in enumerate(sperm):
                cost += _cost(cs[pi]['bb'], ns[gi]['t'], bbody, margin)
                chosen.append((cs[pi], ns[gi]))
            # порядок перебора = порядок в записи: при равной стоимости побеждает первый
            if best is None or cost < best[0] - 1e-9:
                best = (cost, chosen)
    pairs = best[1]
    out = list(tris)
    for c, n in pairs:
        r = _rot(n['rot'])
        for k in c['idx']:
            out[k] = tuple(_apply(v, r, n['t']) for v in tris[k])
    rep['placed'] = len(pairs)
    rep['pairs'] = [(len(c['idx']), n['k']) for c, n in pairs]
    return out, rep


def main(argv):
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    import g101_verts as g
    import lf
    arc = lf.Archive(open(argv[0], 'rb').read())
    want = int(argv[1]) if len(argv) > 1 else None
    for e in arc.entries:
        if want is not None and e.index != want:
            continue
        d = bytes(e.data)
        nodes = read_nodes(d)
        if not nodes:
            continue
        tris = g.entry_mesh(d)[0]
        if not tris:
            continue
        _, rep = place(tris, nodes)
        print('#%d: узлов %d, локальных частей %d, поставлено %d %s' % (e.index, rep['nodes'], rep['local'], rep['placed'], rep['reason']))
        if want is not None:
            for n in nodes:
                print('   узел', n['i'], 'child', n['child'], 'sib', n['sib'], hex(n['id']), ' '.join('%8.3f' % x for x in n['t'] + n['rot']))
            print('   части(треугольников, узел):', rep.get('pairs'))


if __name__ == '__main__':
    main(sys.argv[1:])
