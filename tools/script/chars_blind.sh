#!/bin/sh
# K-190: слепой прогон (PARITY_ENV=ИМЯ=1 — другой флаг, умолчание SUNPETAL_NATIVE_CHARS=1) персонажей — те же сцены и кадры без флага и с SUNPETAL_NATIVE_CHARS=1, потом cmp по сцене.
# Последний столбец строки кадра — хэш 16 записей персонажей и масок «встречен» (session::character_hash).
#   chars_blind.sh ROM SAVE OUTDIR [FRAMES] [СЦЕНА...]
set -e
ROM=$1; SAVE=$2; OUT=$3; FRAMES=${4:-6000}
[ $# -ge 3 ] || { echo "usage: $0 ROM SAVE OUTDIR [FRAMES] [SCENE...]" >&2; exit 2; }
shift 3; [ $# -gt 0 ] && shift
SCENES=${*:-C01_01 C01_12 C02_07 C03_16 C04_06 C05_27 C06_05 C07_36 C08_08 C09_16 C10_12 C11_13 C12_09 C13_02 C14_04 C15_02 C16_01 C17_06 C18_02}
H=$(dirname "$0")
"$H/dialog_reference.sh" "$ROM" "$SAVE" "$OUT/arm" "$FRAMES" $SCENES >/dev/null
env ${PARITY_ENV:-SUNPETAL_NATIVE_CHARS=1} "$H/dialog_reference.sh" "$ROM" "$SAVE" "$OUT/native" "$FRAMES" $SCENES >/dev/null
bad=0
for s in $SCENES; do
  if cmp -s "$OUT/arm/$s.tsv" "$OUT/native/$s.tsv"; then r=равны; else r=РАСХОЖДЕНИЕ; bad=$((bad+1)); fi
  echo "$s $(wc -l <"$OUT/arm/$s.tsv") кадров, хэшей персонажей $(awk -F'\t' '{print $NF}' "$OUT/native/$s.tsv" | uniq | wc -l), $r"
done
echo "расхождений сцен: $bad"
