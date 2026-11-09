#!/bin/bash
# паритет загрузчика по всем сценам CNN_NN: 300 кадров, ARM против SUNPETAL_NATIVE_LOAD=1, 10 процессов
cd /Users/comanche_ak/Developer/games/sunpetal
rm -rf build/pall; mkdir -p build/pall; rm -f build/parity_all.done build/parity_all.txt
ROM="$HOME/Developer/games/fsar-ru/roms/Flower, Sun and Rain - Murder and Mystery in Paradise (Europe) (En,Fr,De,Es,It).nds"
one() {
  s=$1
  for m in arm native; do
    cp build/full100.sav build/pall/$s.$m.sav
    if [ $m = native ]; then E="SUNPETAL_NATIVE_LOAD=1"; else E="X=1"; fi
    printf "goto $s\ndialogref 300 build/pall/$s.$m.tsv\n" | env $E timeout 150 target/release/sunpetal-debug "$ROM" --save build/pall/$s.$m.sav --repl >/dev/null 2>&1
    rm -f build/pall/$s.$m.sav build/pall/$s.$m.sav.lock
  done
  if [ -s build/pall/$s.arm.tsv ] && cmp -s build/pall/$s.arm.tsv build/pall/$s.native.tsv; then echo "$s равны" ; else echo "$s РАСХОЖДЕНИЕ"; fi > build/pall/$s.res
}
for s in $(awk -F, 'NR>1 && $1 ~ /^C[0-9][0-9]_[0-9][0-9]$/ {print $1}' docs/native-scenes.csv); do
  one $s &
  while [ $(jobs -r | wc -l) -ge 10 ]; do sleep 1; done
done
wait
cat build/pall/*.res | awk '{print $2}' | sort | uniq -c > build/parity_all.txt
touch build/parity_all.done
