#!/bin/sh
# K-183: автоматическая часть проверки моделей PS2 (метрики формы, эталонный список, отчёт). Окно не открывает.
# Использование: tools/qa/k183.sh [КАТАЛОГ]   (по умолчанию build/ps2-catalog; нет каталога — строится из build/ps2-models/raw)
# Без образов PS2 (нет ни каталога, ни raw) шаг пропускается с кодом 0.
cd "$(dirname "$0")/../.." || exit 1
CAT=${1:-build/ps2-catalog}; RAW=build/ps2-models/raw
if [ ! -f "$CAT/index.csv" ]; then
  if [ -d "$RAW" ]; then
    echo "== 1. каталог моделей (K-125)"; python3 tools/ps2/catalog.py "$RAW" "$CAT" >"$CAT.log" 2>&1 || { echo "каталог не собрался: $CAT.log" >&2; exit 1; }
  else
    echo "k183: нет каталога ($CAT) и сырых архивов ($RAW), шаг пропущен" >&2; exit 0
  fi
fi
echo "== 2. метрики, статус, эталонный список"
python3 tools/ps2/check_models.py "$CAT" --check --csv "$CAT/index_metrics.csv" --report "$CAT/report.html" | tail -4 || exit 1
echo "== 3. тесты проверок"; python3 -m unittest tools.ps2.test_check_models 2>&1 | tail -3
echo "== Ваша часть: открыть $CAT/report.html — красное и жёлтое сверху; верно ли, что красные действительно сломаны"
