#!/bin/sh
# K-110: автоматическая часть проверки. Окно не открывает; в конце открывает PNG с тремя видами модели.
# Использование: tools/qa/k110.sh [ИМЯ]   (по умолчанию car, манифест tools/ps2/testdata/swap_car.json)
cd "$(dirname "$0")/../.." || exit 1
NAME=${1:-car}; MAN=${MAN:-tools/ps2/testdata/swap_car.json}; D=build/qa-k110
mkdir -p $D
echo "== 1. конвертер"; python3 tools/ps2/mkmodels.py "$MAN" $D "$NAME" || exit 1
echo "== 2. чтение обратно, OBJ, картинка"; python3 tools/qa/k110_check.py "$MAN" $D "$NAME" || exit 1
echo "== 3. секция MODL в файле данных"
cargo build --release -q -p sunpetal-data || exit 1
target/release/sunpetal-pack-models --dir $D --out $D/models.data || exit 1
target/release/sunpetal-check $D/models.data || exit 1
BASE=${BASE:-build/9b/full-ru.data}
if [ -f "$BASE" ]; then
  target/release/sunpetal-pack-models --dir $D --out $D/full-with-models.data --merge "$BASE" || exit 1
  target/release/sunpetal-check $D/full-with-models.data | tail -15 || exit 1
fi
echo "== 4. тесты"; python3 -m unittest discover tools/ps2 2>&1 | tail -3
cargo test -q --release -p sunpetal-data 2>&1 | grep -E "test result|FAILED|failed" 
cargo test -q --release -p sunpetal-app --lib 2>&1 | grep -E "test result|FAILED|failed"
echo "== Ваша часть: посмотреть $D/$NAME.png (форма узнаваема, нет рваных полос)"
open $D/$NAME.png
