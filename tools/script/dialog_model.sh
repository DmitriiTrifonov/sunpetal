#!/bin/sh
# K-177: нативная модель диалога рядом с игрой (`dialogmodel` в sunpetal-debug), слова сверяются каждый кадр.
#   dialog_model.sh ROM SAVE OUTDIR [FRAMES] [СЦЕНА...]
# Вывод: OUTDIR/СЦЕНА.json (кадры, расхождения, первые десять, число TEXT_EX у модели и смен указателя текста у игры).
set -e
ROM=$1; SAVE=$2; OUT=$3; FRAMES=${4:-3000}
[ $# -ge 3 ] || { echo "usage: $0 ROM SAVE OUTDIR [FRAMES] [SCENE...]" >&2; exit 2; }
shift 3; [ $# -gt 0 ] && shift
SCENES=${*:-C01_01 C04_01 C08_01 C12_01 C16_01 C18_01}
mkdir -p "$OUT"
D=$(dirname "$0")/../../target/release/sunpetal-debug
for s in $SCENES; do
  cp "$SAVE" "$OUT/$s.sav"
  ( printf 'goto %s\ndialogmodel %s\n' "$s" "$FRAMES" | "$D" "$ROM" --save "$OUT/$s.sav" --repl 2>/dev/null | tail -1 >"$OUT/$s.json"; rm -f "$OUT/$s.sav" ) &
done
wait
for s in $SCENES; do python3 -c "
import json
d=json.load(open('$OUT/$s.json'))
print('$s', d['frames'], 'расхождений', d['mismatches'], 'TEXT_EX модели', d['texts_model'], 'смен текста у игры', d['texts_game'], 'взято mark у игры', d['resyncs'])"; done
