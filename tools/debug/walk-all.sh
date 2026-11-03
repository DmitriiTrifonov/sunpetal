#!/bin/sh
# Обход вступлений всех глав без окна (K-039): по главе --goto Cnn_01 --autoplay на копии сохранения,
# таблица «глава, код возврата, остановка, реплик, кадр». Использование:
#   tools/debug/walk-all.sh ROM SAVE_С_ОТКРЫТЫМИ_ЗАПРОСАМИ OUTDIR [главы...]
# Сохранение копируется на каждую главу (--save его изменяет), оригинал не трогается.
# Код 1 при падении, зависании (таймаут 300 с) или ошибке запуска главы.
ROM="$1"; SAVE="$2"; OUT="$3"; shift 3
CHAPTERS="${*:-$(seq 1 19)}"
mkdir -p "$OUT"
bad=0
printf '%-7s %-4s %-10s %-6s %s\n' глава код остановка реплик кадр
for c in $CHAPTERS; do
  n=$(printf 'C%02d_01' "$c")
  cp "$SAVE" "$OUT/work.sav"
  timeout 300 target/release/sunpetal-debug "$ROM" --save "$OUT/work.sav" --goto "$n" --autoplay --max-lines 200 >"$OUT/$n.json" 2>"$OUT/$n.err"
  rc=$?
  line=$(tail -1 "$OUT/$n.json")
  info=$(printf '%s' "$line" | python3 -c 'import sys,json
try:
    d=json.loads(sys.stdin.read())
except Exception:
    print("? ? ?"); sys.exit()
st=d.get("state") or {}
print(d.get("stop") or ("ошибка:"+d["error"][:40] if "error" in d else "?"), st.get("lines","-"), d.get("frame","-"))')
  set -- $info
  printf '%-7s %-4s %-10s %-6s %s\n' "$n" "$rc" "$1" "${2:--}" "${3:--}"
  [ "$rc" -ne 0 ] && bad=1
done
rm -f "$OUT/work.sav"
exit $bad
