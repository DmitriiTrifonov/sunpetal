#!/usr/bin/env python3
"""K-193 (Э3): тонкие обработчики команд сценария → таблица данных docs/native-thin-hooks.csv.

Читает сгенерированный C обработчиков (generated/recomp/p_arm9_01[23].c, строки с asm в комментариях) и
символически исполняет Thumb-подмножество: movs/mvns/ldr(литерал, [rb,#imm], [rb,rr])/str*/adds/subs/lsls/muls.
Результат — список записей в память st8/16/32(АДРЕС, ЗНАЧЕНИЕ) с выражениями ld32(..), add(..), sub, mul, shl, not.
Обработчик с ветвлением, вызовом или чтением регистров, не заданных входом, пропускается (в stderr с причиной).
  thin_hooks.py [DISPATCH_CSV] > docs/native-thin-hooks.csv
"""
import csv, glob, re, sys

SKIP_TAGS = {  # уже нативные другим путём (nativedialog / nativestep) или в таблице диалога
    "<TEXT_EX>", "<SHOWWINDOW>", "<HIDEWINDOW>", "<NEXTPAGE>", "<JUMP>", "<SETVALUE>", "<ADDVALUE>", "<CHANGEBYVALUE>",
}
dispatch = sys.argv[1] if len(sys.argv) > 1 else "docs/native-script-dispatch.csv"
src = "".join(open(f).read() for f in sorted(glob.glob("generated/recomp/p_arm9_01[23].c")))


class Unsupported(Exception):
    pass


def num(s):
    return int(s.lstrip("#"), 0)


def run(body):
    regs = {}
    stores = []
    lits = {}
    lines = []
    for l in body.split("\n"):
        m = re.search(r"// ([0-9A-F]{8}) (.*)$", l)
        if m:
            lit = re.search(r"c->r\[\d+\] = (0x[0-9A-Fa-f]+)u;", l)
            lines.append((m.group(2), int(lit.group(1), 16) if lit else None))

    def get(r):
        if r not in regs:
            raise Unsupported("чтение %s до записи" % r)
        return regs[r]

    def addr(base, off):
        return base if off == 0 else ("add(%s,%d)" % (base, off))

    for asm, lit in lines:
        op, _, rest = asm.partition(" ")
        args = [a.strip() for a in re.split(r",\s*(?![^\[]*\])", rest)] if rest else []
        if asm in ("bx lr",):
            return stores
        if op in ("push", "pop"):
            continue
        if op == "movs" and args[1].startswith("#"):
            regs[args[0]] = str(num(args[1]))
        elif op == "mvns":
            regs[args[0]] = "not(%s)" % get(args[1])
        elif op == "ldr" and args[1].startswith("[pc"):
            regs[args[0]] = str(lit)
        elif op in ("ldr", "ldrb", "ldrh", "ldrsb"):
            m = re.match(r"\[(\w+)(?:, (#?\w+))?\]", args[1])
            base, off = get(m.group(1)), m.group(2)
            if off is None:
                a = base
            elif off.startswith("#"):
                a = addr(base, num(off))
            else:
                a = "add(%s,%s)" % (base, get(off))
            size = {"ldr": "ld32", "ldrb": "ld8", "ldrh": "ld16", "ldrsb": "ld8s"}[op]
            regs[args[0]] = "%s(%s)" % (size, a)
        elif op in ("str", "strb", "strh"):
            m = re.match(r"\[(\w+)(?:, (#?\w+))?\]", args[1])
            base, off = get(m.group(1)), m.group(2)
            if off is None:
                a = base
            elif off.startswith("#"):
                a = addr(base, num(off))
            else:
                a = "add(%s,%s)" % (base, get(off))
            stores.append("%s(%s,%s)" % ({"str": "st32", "strb": "st8", "strh": "st16"}[op], a, get(args[0])))
        elif op in ("adds", "subs"):
            f = "add" if op == "adds" else "sub"
            if len(args) == 3:
                a, b = get(args[1]), (str(num(args[2])) if args[2].startswith("#") else get(args[2]))
            else:
                a, b = get(args[0]), (str(num(args[1])) if args[1].startswith("#") else get(args[1]))
            regs[args[0]] = a if (b == "0" and f in ("add", "sub")) else "%s(%s,%s)" % (f, a, b)
        elif op == "lsls":
            regs[args[0]] = "shl(%s,%d)" % (get(args[1]), num(args[2]))
        elif op == "muls":
            regs[args[0]] = "mul(%s,%s)" % (get(args[1]), get(args[0] if len(args) == 2 else args[2]))
        else:
            raise Unsupported(asm)
    raise Unsupported("нет возврата bx lr")


out = ["tag,entry,ops,source"]
noops, skipped = [], []
for r in csv.DictReader(open(dispatch)):
    if not (r["lines"] and int(r["lines"]) <= 20) or r["name"] in SKIP_TAGS:
        continue
    m = re.search(r"void p_arm9_t_%s\(Cpu\* c\) \{(.*?)\n\}" % r["handler"], src, re.S)
    try:
        st = run(m.group(1))
    except Unsupported as e:
        skipped.append("%s: %s" % (r["name"], e))
        continue
    if not st:
        noops.append(r["name"])
        continue
    st = [re.sub(r"\b(\d{5,})\b", lambda m: "0x%X" % int(m.group(1)), x) for x in st]
    out.append("%s,0x%s,%s,handler 0x%s (%s строк)" % (r["name"], format(int(r["handler"], 16) | 1, "08X"), ";".join(st), r["handler"], r["lines"]))
print("\n".join(out))
sys.stderr.write("пустые (bx lr, подмена не нужна): %d %s; пропущено: %d\n" % (len(noops), " ".join(noops), len(skipped)))
for s in skipped:
    sys.stderr.write("  пропуск %s\n" % s)
