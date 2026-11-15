#!/bin/bash
# паритет «всё нативное вместе»: SUNPETAL_NATIVE=1 против ARM по сценам (docs/native-scenes.csv), FRAMES кадров (по умолчанию 1500),
# 10 процессов (TIMEOUT=секунд на запуск, по умолчанию 300; KEEP=1 — не стирать прежние результаты); сводка в build/native_all.txt, расхождения — build/nat_all/*.res. SCENES="C02_01 C05_01" — только эти.
cd /Users/comanche_ak/Developer/games/sunpetal
mkdir -p build/nat_all; [ -n "$KEEP" ] || rm -rf build/nat_all; mkdir -p build/nat_all; rm -f build/native_all.done build/native_all.txt
ROM="$HOME/Developer/games/fsar-ru/roms/Flower, Sun and Rain - Murder and Mystery in Paradise (Europe) (En,Fr,De,Es,It).nds"
FRAMES=${FRAMES:-1500}
one() {
  s=$1
  for m in arm native; do
    cp build/full100.sav build/nat_all/$s.$m.sav
    if [ $m = native ]; then E="SUNPETAL_NATIVE=1"; else E="X=1"; fi
    printf "goto $s\ndialogref $FRAMES build/nat_all/$s.$m.tsv\n" | env $E timeout ${TIMEOUT:-300} target/release/sunpetal-debug "$ROM" --save build/nat_all/$s.$m.sav --repl >/dev/null 2>&1
    rm -f build/nat_all/$s.$m.sav build/nat_all/$s.$m.sav.lock
  done
  if [ -s build/nat_all/$s.arm.tsv ] && cmp -s build/nat_all/$s.arm.tsv build/nat_all/$s.native.tsv; then echo "$s равны" ; else echo "$s РАСХОЖДЕНИЕ"; fi > build/nat_all/$s.res
}
for s in ${SCENES:-$(awk -F, 'NR>1 && $1 ~ /^C[0-9][0-9]_[0-9][0-9]$/ {print $1}' docs/native-scenes.csv)}; do
  one $s &
  while [ $(jobs -r | wc -l) -ge 10 ]; do sleep 1; done
done
wait
cat build/nat_all/*.res | grep -c равны | sed 's/^/равны: /'; cat build/nat_all/*.res | grep -c РАСХ | sed 's/^/расхождений: /' > build/native_all.txt
touch build/native_all.done
