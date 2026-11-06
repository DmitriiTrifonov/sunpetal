#!/usr/bin/env python3
"""K-161: трасса «команда сценария → функции ARM9, которые игра вызвала при её исполнении».

    trace_opcodes.py ROM SAVE OUT.json СЦЕНА[,СЦЕНА…] [КАДРОВ]

Запускает `sunpetal-debug --repl --goto СЦЕНА` (окно не нужно). На каждом кадре читает указатель на текущую
команду (`02075988`), код команды (первое слово структуры), потом прогоняет один кадр с записью вызовов
(`calls 1 FILE`) и приписывает функции той команде, которая была текущей в начале кадра. Функции, которые
вызываются и в кадрах без смены команды (фон), вычитаются. Результат: JSON {код: {функция: число кадров}}.
Только чтение игры; save-файл копируется во временный.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
from collections import Counter, defaultdict

DBG = os.path.join(os.path.dirname(os.path.abspath(__file__)), "../../target/release/sunpetal-debug")


class Repl:
    def __init__(self, rom, save, scene):
        self.p = subprocess.Popen([DBG, rom, "--save", save, "--goto", scene, "--scale", "1", "--repl"],
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.first = True

    def cmd(self, line):
        self.p.stdin.write(line + "\n")
        self.p.stdin.flush()
        while True:
            out = self.p.stdout.readline()
            if not out:
                raise RuntimeError("отладчик завершился")
            if out.startswith("{") and ('"ok"' in out or '"error"' in out or '"frame"' in out):
                # первая строка после запуска — итог --goto, пропускаем её один раз
                if self.first and '"state"' in out:
                    self.first = False
                    continue
                return json.loads(out)

    def peek32(self, addr, n=4):
        r = self.cmd(f"peek {addr:08X}:{n}")
        return bytes.fromhex(r["hex"]) if r.get("ok") else None


def main():
    rom, save, out, scenes = sys.argv[1:5]
    frames = int(sys.argv[5]) if len(sys.argv) > 5 else 3000
    per_op = defaultdict(Counter)
    idle = Counter()
    n_idle = 0
    tmp = tempfile.mkdtemp()
    for scene in scenes.split(","):
        sv = os.path.join(tmp, scene + ".sav")
        shutil.copy(save, sv)
        r = Repl(rom, sv, scene)
        f = os.path.join(tmp, "c.tsv")
        prev = None
        for i in range(frames):
            if i % 40 == 5:
                r.cmd("press A 2")
            cp = r.peek32(0x02075988)
            if cp is None:
                continue
            cmd = int.from_bytes(cp, "little")
            op = None
            if 0x02000000 <= cmd < 0x02400000:
                w = r.peek32(cmd)
                op = int.from_bytes(w, "little") if w else None
            r.cmd(f"calls 1 {f}")
            funcs = Counter()
            for line in open(f):
                p = line.split("\t")
                if p[0] == "9":
                    funcs[p[1]] += 1
            cp2 = r.peek32(0x02075988)
            changed = cp2 is not None and int.from_bytes(cp2, "little") != cmd
            if changed and op is not None and op < 200:
                per_op[op].update(funcs.keys())
            elif not changed:
                idle.update(funcs.keys())
                n_idle += 1
        r.p.stdin.write("quit\n")
        r.p.stdin.flush()
        r.p.wait()
    base = {k for k, v in idle.items() if v >= max(3, n_idle // 2)}
    res = {str(op): {k: v for k, v in c.items() if k not in base} for op, c in sorted(per_op.items())}
    json.dump({"opcodes": res, "baseline": sorted(base), "idle_frames": n_idle}, open(out, "w"), indent=1)
    print(f"кодов {len(res)}, фоновых функций {len(base)}, кадров без смены команды {n_idle}")


main()
