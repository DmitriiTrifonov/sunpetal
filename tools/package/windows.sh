#!/bin/sh
# Собирает build/package/Sunpetal-windows.zip (Sunpetal.exe, иконка, лицензия) кросс-компиляцией с macOS или Linux (K-057).
# Игровой код производный от ROM: пакет делается на машине игрока из его generated/recomp и никому не передаётся.
# Нужны: mingw-w64 (x86_64-w64-mingw32-gcc, windres; macOS: brew install mingw-w64), цель Rust x86_64-pc-windows-gnu
# (rustup target add x86_64-pc-windows-gnu), python3, zip. CONSOLE=1 оставляет окно консоли с журналом.
set -e
cd "$(dirname "$0")/../.."
T=x86_64-pc-windows-gnu
OUT=build/package
W=$OUT/windows
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar
[ -d generated/recomp ] || echo "предупреждение: нет generated/recomp, приложение будет без модулей игры (docs/BUILD.md)" >&2
rm -rf "$W" && mkdir -p "$W/pkg"
python3 tools/android/make-icon.py --ico "$W/sunpetal.ico"
echo '1 ICON "sunpetal.ico"' > "$W/icon.rc"
(cd "$W" && x86_64-w64-mingw32-windres icon.rc -O coff -o icon.o)
SUBSYS=windows; [ -n "$CONSOLE" ] && SUBSYS=console
cargo rustc --release --target $T -p sunpetal-app --bin sunpetal-app -- \
    -C link-arg="$W/icon.o" -C link-arg=-Wl,--subsystem,$SUBSYS
cp target/$T/release/sunpetal-app.exe "$W/pkg/Sunpetal.exe"
cp LICENSE NOTICE "$W/pkg/"
rm -f "$OUT/Sunpetal-windows.zip"
(cd "$W/pkg" && zip -q -9 ../../Sunpetal-windows.zip Sunpetal.exe LICENSE NOTICE)
unzip -tq "$OUT/Sunpetal-windows.zip"
ls -la "$OUT/Sunpetal-windows.zip"
