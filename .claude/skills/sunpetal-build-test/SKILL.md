---
name: sunpetal-build-test
description: Как собрать Sunpetal и прогнать проверки: рекомпиляция ROM в C, самотест, cargo test рантайма, безголовый запуск. Использовать перед сборкой, тестами рантайма и при вопросах «как запустить».
---

# Сборка и проверка

```sh
ROM="$HOME/Developer/games/fsar-ru/roms/Flower, Sun and Rain - Murder and Mystery in Paradise (Europe) (En,Fr,De,Es,It).nds"
export PYTHONPATH=$PWD/tools                      # пакет ndskit лежит в tools/ndskit (форк, эмиттер пишет чистый C)
python3 -m ndskit.recomp "$ROM" --out generated/recomp --optest          # ~45 с, C + векторы optest
python3 -m ndskit.recomp.selftest "$ROM" --cpu 9 --auto --out generated/recomp/selftest/arm9_vectors.txt
for o in 1 3 4 5 6; do python3 -m ndskit.recomp.selftest "$ROM" --cpu 9 --overlay $o --auto --out generated/recomp/selftest/arm9_ov${o}_vectors.txt; done
SUNPETAL_ROM="$ROM" cargo test -p sunpetal-runtime   # ~1,5 мин (первая сборка C)
cargo run --release -p sunpetal-runtime --bin sunpetal-headless -- "$ROM" --vblanks 600   # безголовый запуск
```

Без `generated/recomp` крейт собирается без модулей, тесты самотеста не компилируются.
