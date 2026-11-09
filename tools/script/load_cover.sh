#!/bin/sh
# покрытие загрузчика по всем сценам CNN_NN: 120 кадров на сцену, 10 процессов
cd /Users/comanche_ak/Developer/games/sunpetal
rm -rf build/cva; mkdir -p build/cva; rm -f build/cover_all.done
one() {
  s=$1
  ROM="$HOME/Developer/games/fsar-ru/roms/Flower, Sun and Rain - Murder and Mystery in Paradise (Europe) (En,Fr,De,Es,It).nds"
  cp build/full100.sav build/cva/$s.sav
  printf "goto $s\nrun 120\n" | SUNPETAL_NATIVE_LOAD=1 SUNPETAL_NATIVE_LOAD_LOG=1 timeout 120 target/release/sunpetal-debug "$ROM" --save build/cva/$s.sav --repl 2>&1 >/dev/null | grep "native load\|SETFACE2" | sort -u > build/cva/$s.txt
  rm -f build/cva/$s.sav build/cva/$s.sav.lock
}
export -f one 2>/dev/null
awk -F, 'NR>1 && $1 ~ /^C[0-9][0-9]_[0-9][0-9]$/ {print $1}' docs/native-scenes.csv | while read s; do
  one $s &
  while [ $(jobs -r | wc -l) -ge 10 ]; do sleep 1; done
done
wait
cat build/cva/*.txt | sort | uniq -c | sort -rn > build/cover_all.txt
touch build/cover_all.done
