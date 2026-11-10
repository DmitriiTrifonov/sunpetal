#!/bin/sh
# Собирает build/package/Sunpetal.app и build/package/Sunpetal.dmg из локальной сборки (K-057).
# Игровой код производный от ROM, поэтому образ делается на машине игрока и никому не передаётся
# (как tools/android/build-apk.sh). Подпись ad hoc: хватает для запуска собранного у себя.
# Нужны macOS (hdiutil, iconutil, codesign), Rust. С generated/recomp внутри будет игра, без него приложение без модулей игры.
set -e
cd "$(dirname "$0")/../.."
OUT=build/package
APP=$OUT/Sunpetal.app
VER=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[ -d generated/recomp ] || echo "предупреждение: нет generated/recomp, приложение будет без модулей игры (docs/BUILD.md)" >&2
cargo build --release -p sunpetal-app
rm -rf "$APP" "$OUT/dmg" "$OUT/Sunpetal.iconset" "$OUT/Sunpetal.dmg"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/sunpetal-app "$APP/Contents/MacOS/sunpetal-app"
python3 tools/android/make-icon.py --iconset "$OUT/Sunpetal.iconset"
iconutil -c icns "$OUT/Sunpetal.iconset" -o "$APP/Contents/Resources/Sunpetal.icns"
rm -rf "$OUT/Sunpetal.iconset"
cp LICENSE NOTICE "$APP/Contents/Resources/"
cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Sunpetal</string>
  <key>CFBundleDisplayName</key><string>Sunpetal</string>
  <key>CFBundleIdentifier</key><string>org.sunpetal.app</string>
  <key>CFBundleExecutable</key><string>sunpetal-app</string>
  <key>CFBundleIconFile</key><string>Sunpetal</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VER</string>
  <key>CFBundleVersion</key><string>$VER</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.games</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict></plist>
EOF
codesign --force --deep -s - "$APP"
codesign --verify --deep --strict "$APP"
mkdir -p "$OUT/dmg" && cp -R "$APP" "$OUT/dmg/" && ln -s /Applications "$OUT/dmg/Applications"
hdiutil create -quiet -volname Sunpetal -srcfolder "$OUT/dmg" -ov -format UDZO "$OUT/Sunpetal.dmg"
rm -rf "$OUT/dmg"
hdiutil verify -quiet "$OUT/Sunpetal.dmg"
ls -la "$OUT/Sunpetal.dmg"
