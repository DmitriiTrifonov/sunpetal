#!/bin/sh
# Собирает build/package/Sunpetal-<arch>.AppImage из локальной сборки (K-057).
# Игровой код производный от ROM: образ делается на машине игрока (с его generated/recomp) и никому не передаётся.
#
#   tools/package/linux.sh                  на Linux: собирает здесь (нужны cargo, libasound2-dev, libudev-dev, curl, python3)
#   DOCKER=1 tools/package/linux.sh         на любой системе с Docker: сборка в Ubuntu 22.04 (glibc 2.35, старше — совместимее)
#   DOCKER=1 PLATFORM=linux/arm64 …         другая архитектура (по умолчанию linux/amd64; на Mac с Apple Silicon amd64 идёт
#                                           через эмуляцию и занимает часы, там берите arm64)
#
# appimagetool скачивается закреплённой версией 1.9.1, сумма сверяется (tools/package/appimagetool.sha256).
set -e
cd "$(dirname "$0")/../.."
OUT=build/package

if [ -n "$DOCKER" ] && [ -z "$IN_CONTAINER" ]; then
    PLATFORM=${PLATFORM:-linux/amd64}
    [ -d generated/recomp ] || echo "предупреждение: нет generated/recomp, приложение будет без модулей игры (docs/BUILD.md)" >&2
    mkdir -p "$OUT"
    exec docker run --rm --platform "$PLATFORM" -e IN_CONTAINER=1 -v "$PWD":/src -w /src ubuntu:22.04 sh tools/package/linux.sh
fi

if [ -n "$IN_CONTAINER" ]; then
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq build-essential curl ca-certificates file python3 pkg-config libasound2-dev libudev-dev >/dev/null
    if ! command -v cargo >/dev/null; then
        curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null
    fi
    . "$HOME/.cargo/env"
    # свой каталог сборки: объектные файлы macOS и Linux не смешиваются
    export CARGO_TARGET_DIR=/src/build/package/linux-target-$(uname -m)
fi

ARCH=$(uname -m)   # x86_64 или aarch64
VER=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
TARGET=${CARGO_TARGET_DIR:-target}
APPDIR=$OUT/linux/Sunpetal.AppDir
TOOLDIR=$OUT/linux/tool
[ -d generated/recomp ] || echo "предупреждение: нет generated/recomp, приложение будет без модулей игры (docs/BUILD.md)" >&2

cargo build --release -p sunpetal-app --bin sunpetal-app

rm -rf "$APPDIR" && mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/doc/sunpetal" "$TOOLDIR"
cp "$TARGET/release/sunpetal-app" "$APPDIR/usr/bin/sunpetal-app"
cp LICENSE NOTICE "$APPDIR/usr/share/doc/sunpetal/"
python3 tools/android/make-icon.py --iconset "$OUT/linux/iconset"
cp "$OUT/linux/iconset/icon_256x256.png" "$APPDIR/sunpetal.png"
rm -rf "$OUT/linux/iconset"
cat > "$APPDIR/sunpetal.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Sunpetal
Comment=Native port of a Nintendo DS adventure game, built from your own ROM
Exec=sunpetal-app
Icon=sunpetal
Categories=Game;
Terminal=false
EOF
cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE=$(dirname "$(readlink -f "$0")")
exec "$HERE/usr/bin/sunpetal-app" "$@"
EOF
chmod +x "$APPDIR/AppRun"

TOOL=$TOOLDIR/appimagetool-$ARCH.AppImage
if [ ! -x "$TOOL" ]; then
    curl -sSfL -o "$TOOL" "https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-$ARCH.AppImage"
    WANT=$(grep "appimagetool-$ARCH.AppImage" tools/package/appimagetool.sha256 | cut -d' ' -f1)
    GOT=$(sha256sum "$TOOL" | cut -d' ' -f1)
    [ "$WANT" = "$GOT" ] || { echo "сумма appimagetool не совпала: ждали $WANT, получили $GOT" >&2; rm -f "$TOOL"; exit 1; }
    chmod +x "$TOOL"
fi
# --appimage-extract-and-run: без FUSE (в контейнере его нет)
ARCH=$ARCH "$TOOL" --appimage-extract-and-run "$APPDIR" "$OUT/Sunpetal-$ARCH.AppImage" >/dev/null
ls -la "$OUT/Sunpetal-$ARCH.AppImage"
