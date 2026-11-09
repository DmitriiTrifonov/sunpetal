#!/usr/bin/env python3
"""K-198: таблица функций загрузчика таблицы команд из сгенерированного C -> docs/native-load-hooks.csv.
    load_hooks.py [OUT.csv]
Вид `skip`: функции формы «читать токены, пока текст токена не равен закрывающему тегу, затем счётчик записей +1»
(кластер 1 из load_shapes.py, 25 строк): константы берутся из C по месту.
Столбцы: key (адрес | 1), kind, close (строка закрывающего тега), base (состояние текста), off (смещение указателя на текст),
state (состояние записи), idx (смещение индекса записи), n0 (начальный счётчик сравнения r4), ret (адрес возврата вызова лексера)."""
import re, sys, collections, glob
sys.path.insert(0, __file__.rsplit('/', 1)[0])
import load_shapes as L


def bodies():
    out = {}
    for f in sorted(glob.glob('generated/recomp/p_arm9_01*.c')):
        cur = None
        for l in open(f):
            m = re.match(r'void p_arm9_t_([0-9A-F]{8})\(Cpu\* c\) \{', l)
            if m:
                cur = m.group(1); out[cur] = []; continue
            if l.startswith('}'): cur = None; continue
            if cur: out[cur].append(l)
    return out


def skip_row(a, ls):
    t = ''.join(ls)
    g = lambda p: re.search(p, t)
    close = g(r'c->r\[7\] = (0x[0-9A-F]+)u;')
    base = g(r'c->r\[5\] = (0x[0-9A-F]+)u;')
    off = g(r'c->r\[3\] = ldr32\(c, c->r\[5\] \+ (0x[0-9A-F]+)u\)')
    state = g(r'c->r\[0\] = (0x[0-9A-F]+)u;[^\n]*\n[^\n]*c->r\[1\] = ldr32\(c, c->r\[0\] \+ (0x[0-9A-F]+)u\)')
    n0 = g(r'c->r\[4\] = r;[^\n]*// [0-9A-F]+ movs r4, #(\d+)') or g(r'const uint32_t b = (0x[0-9a-f]+)u; const uint32_t r = b; c->r\[4\] = r')
    ret = g(r'c->r\[14\] = (0x[0-9A-F]+)u; ARM_POLL\(c\); p_arm9_t_0203F9D8')
    if not all((close, base, off, state, ret)):
        return None
    n = int(n0.group(1), 0) if n0 else 0
    return [a, 'skip', close.group(1), base.group(1), off.group(1), state.group(1), state.group(2), str(n), ret.group(1)]


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else 'docs/native-load-hooks.csv'
    F, B = L.funcs(), bodies()
    cl = collections.defaultdict(list)
    for a, ins in F.items():
        if '0203F024' <= a <= '02044750' and any('bl 0x0203F9D8' in i for _, i in ins):
            cl[L.shape(ins)].append(a)
    first = max(cl.values(), key=len)
    rows = []
    for a in first:
        r = skip_row(a, B[a])
        if r is None:
            print('пропуск', a, file=sys.stderr)
        else:
            r[0] = '0x%08X' % (int(a, 16) | 1)
            rows.append(r)
    with open(out, 'w') as f:
        f.write('key,kind,close,base,off,state,idx,n0,ret\n')
        for r in rows:
            f.write(','.join(r) + '\n')
    print(len(rows), 'строк ->', out)


if __name__ == '__main__':
    main()
