#!/bin/sh
# K-188 (Э1.1): инвентаризация ARM — `calls` по пяти сценариям, потом arm_inventory.py.
#   arm_inventory.sh ROM SAVE [OUTDIR]     (OUTDIR по умолчанию build/native-engine; нужны generated/recomp и release-сборка sunpetal-debug)
# Вывод: docs/native-engine-inventory.csv, docs/NATIVE-ENGINE-INVENTORY.md; сырые calls_*.tsv — в OUTDIR (не в git).
set -e
ROM=$1; SAVE=$2; O=${3:-build/native-engine}
[ $# -ge 2 ] || { echo "usage: $0 ROM SAVE [OUTDIR]" >&2; exit 2; }
cd "$(dirname "$0")/../.."
D=target/release/sunpetal-debug
T=$(mktemp -d); mkdir -p "$O"
for n in dialog walk menu device title; do cp "$SAVE" "$T/$n.sav"; done
r() { n=$1; shift; "$D" "$ROM" --save "$T/$n.sav" --scale 1 --repl "$@" >"$T/$n.log" 2>&1; }
(printf '%s\n' "calls 3000 $O/calls_dialog.tsv" quit | r dialog --goto C01_01) &
(printf '%s\n' "skip 100" "press RIGHT 60" "calls 600 $O/calls_walk.tsv" quit | r walk --goto C02_01 --autoplay) &
(printf '%s\n' "skip 100" "press START 3" "advance 40" "calls 300 $O/calls_menu.tsv" quit | r menu --goto C02_01 --autoplay) &
(printf '%s\n' "skip 100" "press START 3" "advance 40" "tap 25 43 4" "advance 100" "calls 300 $O/calls_device.tsv" quit | r device --goto C02_01 --autoplay) &
(printf '%s\n' "advance 1500" "calls 600 $O/calls_title.tsv" quit | r title) &
wait
rm -rf "$T"
python3 -I tools/script/arm_inventory.py "$O" docs/native-engine-inventory.csv "$O/summary.md" >/dev/null
cp "$O/summary.md" docs/NATIVE-ENGINE-INVENTORY.md
wc -l docs/native-engine-inventory.csv
