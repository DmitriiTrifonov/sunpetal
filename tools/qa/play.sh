#!/bin/sh
# Запуск окна игры для проверок docs/qa. Не трогает уже открытое окно.
# Использование: tools/qa/play.sh [ru|en] [аргументы sunpetal-app…]
# Переменные: SUNPETAL_PACK_2D=<папка или файл пакета 2D> и любые SUNPETAL_* уходят в игру как есть.
cd "$(dirname "$0")/../.." || exit 1
if pgrep -fl sunpetal-app; then
  echo "Окно уже запущено: закройте его сами или скажите ассистенту «перезапуск»"; exit 1
fi
LANG_ARG=${1:-ru}; [ $# -gt 0 ] && shift
# en = текст ROM без перевода
[ "$LANG_ARG" = en ] && LANG_ARG=none
cargo build --release -p sunpetal-app || exit 1
ROM="${SUNPETAL_ROM:-$HOME/Developer/games/fsar-ru/roms/Flower, Sun and Rain - Murder and Mystery in Paradise (Europe) (En,Fr,De,Es,It).nds}"
mkdir -p build artifacts/shots build/qa-config
# Свой конфиг для проверок: копия пользовательского, но texture_pack = файл данных с переводом
# (пакет только с текстурами в texture_pack ломает запуск с языком, K-158). Пользовательский конфиг не трогаем.
DATA="${SUNPETAL_QA_DATA:-build/9b/full-ru.data}"
[ -f "$DATA" ] || { echo "Нет файла данных $DATA (задайте SUNPETAL_QA_DATA=…)"; exit 1; }
USERCFG="${SUNPETAL_CONFIG:-$HOME/Library/Application Support/sunpetal}/config.toml"
if [ -f "$USERCFG" ]; then
  sed "s|^texture_pack = .*|texture_pack = \"$DATA\"|" "$USERCFG" > build/qa-config/config.toml
else
  rm -f build/qa-config/config.toml
fi
SUNPETAL_CONFIG="$PWD/build/qa-config" SUNPETAL_RAM_DUMP=1 nohup target/release/sunpetal-app "$ROM" --lang "$LANG_ARG" "$@" \
  > build/qa-app.log 2>&1 < /dev/null &
sleep 2; pgrep -fl sunpetal-app || { echo "Не запустилось, см. build/qa-app.log"; exit 1; }
echo "Снимки по F10: artifacts/shots/, лог: build/qa-app.log"
