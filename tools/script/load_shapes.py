#!/usr/bin/env python3
"""K-198: формы функций загрузчика таблицы команд — кластеры по последовательности команд без констант.
    load_shapes.py [-v]   кластеры, размер, адреса; -v — показывает константы-отличия первого и второго в кластере"""
import glob, re, sys, collections

def funcs():
    out = {}
    for f in sorted(glob.glob('generated/recomp/p_arm9_01*.c')):
        cur = None
        for l in open(f):
            m = re.match(r'void p_arm9_t_([0-9A-F]{8})\(Cpu\* c\) \{', l)
            if m:
                cur = m.group(1); out[cur] = []; continue
            if l.startswith('}'): cur = None; continue
            if cur:
                m = re.search(r'// ([0-9A-F]{8}) (.*)$', l)
                if m: out[cur].append((m.group(1), m.group(2)))
    return out

def shape(ins):
    s = []
    for a, i in ins:
        i = re.sub(r'0x[0-9a-fA-F]+', 'K', i)
        i = re.sub(r'#-?\d+', '#K', i)
        s.append(i)
    return tuple(s)

if __name__ == '__main__':
    F = funcs()
    cl = collections.defaultdict(list)
    for a, ins in F.items():
        if '0203F024' <= a <= '02044750' and any('bl 0x0203F9D8' in i for _, i in ins):
            cl[shape(ins)].append(a)
    rows = sorted(cl.values(), key=lambda v: -len(v))
    for v in rows:
        print(len(v), len(F[v[0]]), ' '.join(v))
    print('clusters', len(rows), 'functions', sum(len(v) for v in rows))
