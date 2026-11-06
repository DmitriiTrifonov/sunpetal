#!/usr/bin/env python3
"""K-169: как игра читает ввод при ходьбе. Прогон через REPL sunpetal-debug: герой на свободной ходьбе C02_01,
кнопка или касание нижнего экрана (круглая карта), смещение героя по X/Z (1/4096 единицы).
Запуск: python3 -I tools/script/walk_probe.py ROM [SAVE]   (из корня репозитория, нужен target/release/sunpetal-debug)
Герой: *(*0x0206B460), позиция в +0x18 (три i32). Центр карты на нижнем экране ~(128, 97), радиус ~82."""
import json, math, struct, subprocess, sys

rom = sys.argv[1]
save = sys.argv[2] if len(sys.argv) > 2 else "build/full100.sav"
scene = sys.argv[3] if len(sys.argv) > 3 else "C02_01"
only = sys.argv[4] if len(sys.argv) > 4 else ""  # "r": только радиусы
# scene "continue": без --goto, титул → Continue первого слота (сохранение игрока)
args = ["--scale", "1", "--repl"] if scene == "continue" else ["--goto", scene, "--autoplay", "--scale", "1", "--repl"]
p = subprocess.Popen(["target/release/sunpetal-debug", rom, "--save", save] + args,
                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
if scene != "continue":
    p.stdout.readline()  # результат goto


def cmd(c):
    p.stdin.write(c + "\n"); p.stdin.flush()
    return json.loads(p.stdout.readline())


def word(a):
    return struct.unpack("<I", bytes.fromhex(cmd(f"peek {a:08X}:4")["hex"]))[0]


if scene == "continue":
    cmd("advance 1500"); cmd("press START 3"); cmd("advance 120"); cmd("tap 190 50 4"); cmd("advance 90")
    cmd("advance 900")
    for _ in range(80):  # A по вступлению, пока не свободная ходьба (герой есть и скрипт простаивает)
        h = word(word(0x0206B460))
        if h and cmd("skip 300").get("stopped") == "idle" and word(word(0x0206B460)):
            break
        cmd("press A 3"); cmd("advance 120")
else:
    cmd("skip 100")
cmd("advance 20")
if scene == "continue":
    cmd("shot artifacts/screens/k169-parking.png")
hero = word(0x0226A150)
pos = lambda: struct.unpack("<3i", bytes.fromhex(cmd(f"peek {hero + 0x18:08X}:12")["hex"]))
start = pos()


def probe(c, frames=20):
    cmd("teleport %d %d %d" % start); cmd("advance 40")
    a = pos(); cmd(f"{c} {frames}"); cmd("advance 10"); b = pos()
    d = (b[0] - a[0], b[2] - a[2])
    return d, round(math.hypot(*d)), round(math.degrees(math.atan2(-d[1], d[0])))


if only != "r":
  print("D-pad")
  for k in ("UP", "UP+RIGHT", "RIGHT", "DOWN+RIGHT", "DOWN", "DOWN+LEFT", "LEFT", "UP+LEFT"):
    print(k, *probe("press " + k))
  print("касание по кругу r=50")
  for deg in range(0, 360, 30):
    x, y = round(128 + 50 * math.cos(math.radians(deg))), round(97 - 50 * math.sin(math.radians(deg)))
    print(deg, (x, y), *probe(f"tap {x} {y}"))
if only == "z":  # зоны шаг/бег на парковке (scene = continue, сохранение игрока): телепорт назад, короткие нажатия
    def zone(c, fr=10):
        cmd("teleport %d %d %d" % start); cmd("advance 60")
        a = pos(); cmd(f"{c} {fr}"); cmd("advance 4"); b = pos()
        return round(math.hypot(b[0] - a[0], b[2] - a[2]))
    for k in ("RIGHT", "A+RIGHT", "B+RIGHT", "R+RIGHT"):
        print("press", k, zone("press " + k))
    for r in (5, 20, 36, 40, 42, 44, 60, 75):
        print("tap r =", r, zone(f"tap {128 + r} 97"))
    p.stdin.write("quit\n"); sys.exit()
print("радиус (вверх от центра), 20 и 60 кадров")
for r in (5, 15, 25, 35, 45, 60, 80):
    print(r, *probe(f"tap 128 {97 - r}"), *probe(f"tap 128 {97 - r}", 60)[1:2])
p.stdin.write("quit\n")
