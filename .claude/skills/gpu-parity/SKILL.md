---
name: gpu-parity
description: Паритет GPU-бэкенда 3D с софтовым: дампы кадров, тесты с фичей gpu, слепой прогон 30 000 кадров, бенчмарки, переменные SUNPETAL_GPU_TIMING/SUNPETAL_TILED/SUNPETAL_JOB_DUMPS. Использовать при любой правке render3d_gpu.rs, render3d.rs или шейдеров.
---

# Паритет GPU и софта

GPU обязан совпадать с софтом слово в слово; рассинхрон — баг GPU.

Паритет GPU и софта (нужна фича `gpu`, дампы кадров):

```sh
export SUNPETAL_JOB_DUMPS=$PWD/build/gpu-parity/dumps          # абсолютный путь
cargo test -p sunpetal-runtime --release --features gpu --lib   # 61 тест, должны проходить все
cargo test -p sunpetal-runtime --release --features gpu --lib supersampling_on_dumps -- --ignored   # 32 проверки
SUNPETAL_ROM="$ROM" cargo test -p sunpetal-runtime --release --features gpu --test boot             # кадр в игре, GPU = софт
SUNPETAL_BENCH_DUMP=$PWD/build/gpu-parity/dumps/c18_02_hotel_002521.spjob cargo test -p sunpetal-runtime --release --features gpu --lib bench_supersampling -- --ignored --nocapture
SUNPETAL_ROM="$ROM" python3 tools/gpu-parity/bench_fps.py --vblanks 6000 --scales 1,2 --msaa 0,4   # кадров/с на живой игре
```

Переменные GPU-пути: `SUNPETAL_GPU_TIMING` (отчёт по кадру: атлас, ресурсы, проходы), `SUNPETAL_TILED=0` (вернуть старый последовательный проход прозрачных, по умолчанию тайлы), `SUNPETAL_JOB_DUMPS` (писать/читать дампы). Слепой прогон: `target/release/sunpetal-headless "$ROM" --vblanks 30000 --draw-all --renderer gpu --input "$(cat build/gpu-parity/blind.input)" --save …`, лог `stderr` сравнить с софтовым (`diff`).

Ограничения на macOS: `ndsrun` из `ndskit` не собирается (нужен OpenGL 4.5). Это нормально: графику пишем сами. Воспроизведение исходного прогона `ndskit` (C++, Python-пакет в `~/Developer/games/ref`) — в `docs/RECON-NDSKIT.md`.
