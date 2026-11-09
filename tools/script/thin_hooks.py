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


COND = {"eq": "eq", "ne": "ne", "lt": "lt", "le": "le", "gt": "gt", "ge": "ge", "lo": "lo", "cc": "lo", "hs": "hs", "cs": "hs", "hi": "hi", "ls": "ls"}


def parse_lines(body):
    lines = []
    for l in body.split("\n"):
        m = re.search(r"// ([0-9A-F]{8}) (.*)$", l)
        if m:
            lit = re.search(r"c->r\[\d+\] = (0x[0-9A-Fa-f]+)u;", l)
            lines.append((int(m.group(1), 16), m.group(2), int(lit.group(1), 16) if lit else None))
    return lines


def run(body):
    """Символическое исполнение с ветвлением вперёд. Возвращает список операторов (строки) в синтаксисе таблицы."""
    lines = parse_lines(body)
    index = {a: i for i, (a, _, _) in enumerate(lines)}

    def mem(base_, off):
        if off is None:
            return base_
        return base_ if off == 0 else "add(%s,%d)" % (base_, off)

    def walk(i, regs, cmp):
        regs = dict(regs)
        out = []
        steps = 0

        def get(r):
            if r not in regs:
                raise Unsupported("чтение %s до записи" % r)
            return regs[r]

        def val(a):
            return str(num(a)) if a.startswith("#") else get(a)

        while True:
            steps += 1
            if steps > 200 or i >= len(lines):
                raise Unsupported("нет возврата")
            addr, asm, lit = lines[i]
            op, _, rest = asm.partition(" ")
            args = [a.strip() for a in re.split(r",\s*(?![^\[{]*[\]}])", rest)] if rest else []
            nxt = lines[i + 1][0] if i + 1 < len(lines) else None
            if op in ("movs", "mvns", "adds", "subs", "lsls", "lsrs", "muls") and cmp is not None:
                cmp = None  # these set the flags: the compare is gone
            if asm == "bx lr" or (op == "pop" and "pc" in rest):
                return out
            if op in ("push", "pop"):
                i += 1
                continue
            if op == "movs" and args[1].startswith("#"):
                regs[args[0]] = str(num(args[1]))
            elif op == "mvns":
                regs[args[0]] = "not(%s)" % get(args[1])
            elif op == "ldr" and args[1].startswith("[pc"):
                regs[args[0]] = str(lit)
            elif op in ("ldr", "ldrb", "ldrh", "ldrsb"):
                m = re.match(r"\[(\w+)(?:, (#?\w+))?\]", args[1])
                base_, off = get(m.group(1)), m.group(2)
                if off is None:
                    a_ = base_
                elif off.startswith("#"):
                    a_ = mem(base_, num(off))
                else:
                    a_ = "add(%s,%s)" % (base_, get(off))
                regs[args[0]] = "%s(%s)" % ({"ldr": "ld32", "ldrb": "ld8", "ldrh": "ld16", "ldrsb": "ld8s"}[op], a_)
            elif op in ("str", "strb", "strh"):
                m = re.match(r"\[(\w+)(?:, (#?\w+))?\]", args[1])
                base_, off = get(m.group(1)), m.group(2)
                if off is None:
                    a_ = base_
                elif off.startswith("#"):
                    a_ = mem(base_, num(off))
                else:
                    a_ = "add(%s,%s)" % (base_, get(off))
                out.append("%s(%s,%s)" % ({"str": "st32", "strb": "st8", "strh": "st16"}[op], a_, get(args[0])))
            elif op in ("adds", "subs"):
                f = "add" if op == "adds" else "sub"
                if len(args) == 3:
                    a_, b_ = get(args[1]), val(args[2])
                else:
                    a_, b_ = get(args[0]), val(args[1])
                regs[args[0]] = a_ if b_ == "0" else "%s(%s,%s)" % (f, a_, b_)
            elif op in ("lsls", "lsrs"):
                regs[args[0]] = "%s(%s,%d)" % ("shl" if op == "lsls" else "shr", get(args[1]), num(args[2]))
            elif op == "muls":
                regs[args[0]] = "mul(%s,%s)" % (get(args[1]), get(args[0] if len(args) == 2 else args[2]))
            elif op == "cmp":
                cmp = (get(args[0]), val(args[1]))
            elif op == "b" and rest.startswith("0x"):
                t = int(rest, 16)
                if t <= addr:
                    raise Unsupported("переход назад")
                i = index[t]
                continue
            elif op[0] == "b" and op[1:] in COND and rest.startswith("0x"):
                if cmp is None:
                    raise Unsupported("ветвление без действующего cmp")
                t = int(rest, 16)
                if t <= addr:
                    raise Unsupported("переход назад")
                taken = walk(index[t], regs, cmp)
                other = walk(i + 1, regs, cmp)
                out.append("if(%s(%s,%s)){%s}else{%s}" % (COND[op[1:]], cmp[0], cmp[1], ";".join(taken), ";".join(other)))
                return out
            elif op in ("bl", "blx") and rest.startswith("0x"):
                t = int(rest, 16) | (1 if op == "bl" else 0)
                argv = [regs.get("r%d" % k, "0") for k in range(4)]
                out.append("call(0x%X,0x%X,%s)" % (nxt | 1, t, ",".join(argv)))
                for k in range(1, 4):
                    regs.pop("r%d" % k, None)
                regs["r0"] = "ret"
            elif op == "blx" and rest.startswith("r"):
                argv = [regs.get("r%d" % k, "0") for k in range(4)]
                out.append("call(0x%X,%s,%s)" % (nxt | 1, get(rest), ",".join(argv)))
                for k in range(1, 4):
                    regs.pop("r%d" % k, None)
                regs["r0"] = "ret"
            elif op == "bx" and rest.startswith("r"):
                argv = [regs.get("r%d" % k, "0") for k in range(4)]
                out.append("tail(%s,%s)" % (get(rest), ",".join(argv)))
                return out
            else:
                raise Unsupported(asm)
            i += 1

    return walk(0, {}, None)


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
    st = [re.sub(r"(?<![0-9A-Fa-fx])(\d{5,})\b", lambda m: "0x%X" % int(m.group(1)), x) for x in st]
    out.append("%s,0x%s,%s,handler 0x%s (%s строк)" % (r["name"], format(int(r["handler"], 16) | 1, "08X"), ";".join(st), r["handler"], r["lines"]))
print("\n".join(out))
sys.stderr.write("пустые (bx lr, подмена не нужна): %d %s; пропущено: %d\n" % (len(noops), " ".join(noops), len(skipped)))
for s in skipped:
    sys.stderr.write("  пропуск %s\n" % s)
