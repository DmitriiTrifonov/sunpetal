#!/usr/bin/env bash
# Единая проверка Sunpetal. Уровни:
#   quick (по умолчанию) — без ROM: сборка, cargo test --lib, fmt и clippy (советом, см. STRICT).
#   full — quick + тесты с ROM (самотест рантайма) и паритет GPU; нужны ROM, generated/recomp, адаптер.
# STRICT=1 — fmt и clippy становятся обязательными. Пока код не отформатирован (1300+ правок fmt,
# ~100 замечаний clippy), они только печатаются: иначе проверка не проходила бы никогда.
# Запуск из любого каталога: tools/verify.sh [quick|full]
set -u
cd "$(dirname "$0")/.."
level="${1:-quick}"
fail=0
step() { echo "== $*" >&2; }
must() { step "$*"; "$@" >&2 || { echo "ПРОВАЛ: $*" >&2; fail=1; }; }
advice() {
  step "$*"
  if ! "$@" >&2; then
    if [ "${STRICT:-0}" = 1 ]; then echo "ПРОВАЛ: $*" >&2; fail=1
    else echo "(совет, не блокирует; STRICT=1 сделает обязательным): $*" >&2; fi
  fi
}

must cargo build --workspace
must cargo test --workspace --lib
advice cargo fmt --all --check
advice cargo clippy --workspace --all-targets -- -D warnings

if [ "$level" = full ]; then
  rom="${SUNPETAL_ROM:-$HOME/Developer/games/fsar-ru/roms/Flower, Sun and Rain - Murder and Mystery in Paradise (Europe) (En,Fr,De,Es,It).nds}"
  [ -f "$rom" ] || { echo "full: нет ROM ($rom)" >&2; exit 1; }
  [ -d generated/recomp ] || { echo "full: нет generated/recomp (скилл sunpetal-build-test)" >&2; exit 1; }
  SUNPETAL_ROM="$rom" must cargo test -p sunpetal-runtime
  # Паритет GPU с софтом, слепой прогон 30000 кадров: скилл gpu-parity (при правках render3d*/шейдеров).
  echo "full: паритет GPU запускай по скиллу gpu-parity, если правились render3d*.rs или шейдеры" >&2
fi

[ "$fail" = 0 ] && echo "verify $level: ок" >&2
exit "$fail"
