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


ARGS_CONSTS = {'G': '0x02074084', 'T': '0x02074BEC', 'N': '0x02065568', 'M': '0x020757EC', 'close': '0x020740EC'}


def args_row_base(a, ins, ls):
    """Вид `args`: прочитать число, при первом круге записать пары (слово состояния +0x50.. -> смещение записи), конец по закрывающему тегу.
    Константы по порядку ldr [pc]: G, T, текст, закрывающий, S, T, N, M, G; роли проверяются на равенство известным."""
    t = ''.join(ls)
    cs = [(int(r), v) for r, v, _ in re.findall(r'c->r\[(\d+)\] = (0x[0-9A-F]{8})u;\s+// ([0-9A-F]+) ldr', t)]
    rets = re.findall(r'r\[14\] = (0x[0-9A-F]+)u; ARM_POLL\(c\); p_arm9_[tf]_(0203F9D8|0203F804|020220F4)', t)
    rets = {f: r for r, f in rets}
    if len(cs) < 9 or len(rets) != 3:
        return None
    # порядок констант: у кластеров 2–3 первая — G (или r7 = G), вторая — T; проверка по значениям
    g, tt = ARGS_CONSTS['G'][2:], ARGS_CONSTS['T'][2:]
    order = [v[2:] for _, v in cs]
    if not (order[0] == g and order[1] == tt and order[3] == ARGS_CONSTS['close'][2:] and order[5] == tt
            and order[6] == ARGS_CONSTS['N'][2:] and order[7] == ARGS_CONSTS['M'][2:] and order[4] == g and order[8] == g):
        return None
    base = '0x' + order[2]
    # смещение указателя на текст: первое ldr r3,[r0,#off] после bl 0x0203F804
    asm = [i for _, i in ins]
    k804 = next(n for n, i in enumerate(asm) if i == 'bl 0x0203F804')
    off = next(hex(int(re.search(r'#(0x[0-9a-f]+|\d+)', i).group(1), 0)) for i in asm[k804:] if i.startswith('ldr r3, [r0, #'))
    k9d8 = next(n for n, i in enumerate(asm) if i == 'bl 0x0203F9D8')
    imm = lambda x: hex(int(x, 0))
    clears = [imm(re.search(r'#(0x[0-9a-f]+|\d+)', i).group(1)) for i in asm[:k9d8] if i.startswith('str ') and 'sp' not in i and '#' in i]
    # после цикла сравнения (blt): пары чтение состояния +0x50.. -> запись
    kb = max(n for n, i in enumerate(asm) if i.startswith('blt '))
    cur, pairs = {}, []
    for i in asm[kb:]:
        m = re.match(r'ldr (r\d), \[r\d, #(0x5[0-9a-f])\]', i)
        if m:
            cur[m.group(1)] = m.group(2)
        m = re.match(r'str (r\d), \[r\d, #(0x[0-9a-f]+|\d+)\]', i)
        if m and m.group(1) in cur:
            pairs.append('%s:%s:1' % (cur[m.group(1)], imm(m.group(2))))
    return [a, 'args', ARGS_CONSTS['close'], base, off, ' '.join(clears), ' '.join(pairs),
            rets['0203F9D8'], rets['0203F804'], rets['020220F4'], 'once',
            ' '.join(re.findall(r'r(\d)', asm[0].split('{')[1]))]


def args_row(a, ins, ls):
    return args_row_base(a, ins, ls)


def seq_row(a, ins, ls):
    """Вид `seq`: n-е число в n-е слово записи (таблица переходов или цепочка сравнений); у каждого случая свой флаг «получено»."""
    r = args_row_base(a, ins, ls)
    if r is None:
        return None
    asm = [i for _, i in ins]
    kb = max(n for n, i in enumerate(asm) if i.startswith('blt '))
    start = next((n for n, i in enumerate(asm) if n > kb and i.startswith('add pc')), None)
    if start is None:
        start = next(n for n, i in enumerate(asm) if n > kb and i.startswith('b 0x'))
    imm = lambda x: hex(int(x, 0))
    cases, cur, st, got = [], None, None, 0
    for i in asm[start + 1:]:
        m = re.match(r'ldr (r\d), \[r\d, #(0x5[0-9a-f])\]', i)
        if m:
            cur, st = m.group(1), m.group(2)
        m = re.match(r'str (r\d), \[r\d, #(0x[0-9a-f]+|\d+)\]', i)
        if m and m.group(1) == cur:
            rec = imm(m.group(2))
        if re.match(r'movs r\d, #1$', i):
            got = 1
        if i.startswith('b 0x'):
            cases.append('%s:%s:%d' % (st, rec, got))
            cur, st, got = None, None, 0
    r[6] = ' '.join(cases)
    r[10] = 'seq'
    return r


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else 'docs/native-load-hooks.csv'
    out2 = out.replace('hooks', 'args')
    F, B = L.funcs(), bodies()
    cl = collections.defaultdict(list)
    for a, ins in F.items():
        if '0203F024' <= a <= '02044750' and any('bl 0x0203F9D8' in i for _, i in ins):
            cl[L.shape(ins)].append(a)
    groups = sorted(cl.values(), key=lambda v: -len(v))
    rows = []
    for a in groups[0]:
        r = skip_row(a, B[a])
        if r is None:
            print('пропуск skip', a, file=sys.stderr)
        else:
            r[0] = '0x%08X' % (int(a, 16) | 1)
            rows.append(r)
    with open(out, 'w') as f:
        f.write('key,kind,close,base,off,state,idx,n0,ret\n')
        for r in rows:
            f.write(','.join(r) + '\n')
    arows = []
    for gi, g in enumerate(groups[1:5]):
        for a in g:
            r = (args_row if gi < 3 else seq_row)(a, F[a], B[a])
            if r is None:
                print('пропуск args', a, file=sys.stderr)
            else:
                r[0] = '0x%08X' % (int(a, 16) | 1)
                arows.append(r)
    with open(out2, 'w') as f:
        f.write('key,kind,close,base,off,clears,pairs,ret_lex,ret_val,ret_warn,mode,push\n')
        for r in arows:
            f.write(','.join(r) + '\n')
    print(len(rows), 'skip ->', out, ';', len(arows), 'args ->', out2)


if __name__ == '__main__':
    main()
