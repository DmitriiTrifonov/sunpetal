#!/bin/sh
# K-176: эталон паритета диалога — состояние и хэш кадров ARM по сценам (`dialogref` в sunpetal-debug).
#   dialog_reference.sh ROM SAVE OUTDIR [FRAMES] [СЦЕНА...]
# Сцены по умолчанию — начала глав с разговорами; по сцене один процесс, параллельно. Вывод: OUTDIR/СЦЕНА.tsv
# (кадр, сцена, хэш обоих экранов, слова состояния: text f48 f4c w2 w74 mark window). Копия сохранения своя у каждой сцены.
set -e
ROM=$1; SAVE=$2; OUT=$3; FRAMES=${4:-3000}
[ $# -ge 3 ] || { echo "usage: $0 ROM SAVE OUTDIR [FRAMES] [SCENE...]" >&2; exit 2; }
shift 3; [ $# -gt 0 ] && shift
SCENES=${*:-C01_01 C04_01 C08_01 C12_01 C16_01 C18_01}
mkdir -p "$OUT"
D=$(dirname "$0")/../../target/release/sunpetal-debug
for s in $SCENES; do
  cp "$SAVE" "$OUT/$s.sav"
  ( printf 'goto %s\ndialogref %s %s\n' "$s" "$FRAMES" "$OUT/$s.tsv" | "$D" "$ROM" --save "$OUT/$s.sav" --repl >"$OUT/$s.log" 2>/dev/null; rm -f "$OUT/$s.sav" ) &
done
wait
wc -l "$OUT"/*.tsv
