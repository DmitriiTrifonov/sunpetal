#!/bin/sh
# Обход глав одним экраном (SINGLE-SCREEN Г.1): по главе — реплики автоплеем, на каждой оба экрана,
# затем отчёт правила выбора экрана. Использование: tools/screen-walk.sh ROM SAVE OUTDIR [главы...]
ROM="$1"; SAVE="$2"; OUT="$3"; shift 3
CHAPTERS="${*:-$(seq 1 20)}"
mkdir -p "$OUT"
for c in $CHAPTERS; do
  n=$(printf 'C%02d_01' "$c")
  target/release/sunpetal-debug "$ROM" --save "$SAVE" --goto "$n" --autoplay --scale 1 --max-lines 60 --frame-delay 60 --screens-dir "$OUT" >"$OUT/$n.json" 2>/dev/null || echo "$n: не запустилась"
done
target/release/sunpetal-screen-report "$OUT"
