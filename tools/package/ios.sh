#!/bin/sh
# Собирает build/package/Sunpetal.ipa (arm64, без подписи) из локальной сборки (K-057).
# Для сайдлоада (AltStore, Sideloadly): они подписывают личным сертификатом при установке; в App Store не выкладываем.
# Игровой код производный от ROM, поэтому образ делается на машине игрока и никому не передаётся.
# Нужны macOS с Xcode (SDK iPhoneOS; если xcode-select смотрит на Command Line Tools, скрипт берёт /Applications/Xcode.app),
# цель Rust aarch64-apple-ios (rustup target add aarch64-apple-ios), python3, zip.
# ROM или sunpetal.data игрок кладёт в папку приложения: «Файлы» → «На iPhone» → Sunpetal (или Finder → iPhone → Файлы).
set -e
cd "$(dirname "$0")/../.."
[ -d /Applications/Xcode.app ] && export DEVELOPER_DIR=${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}
OUT=build/package
W=$OUT/ios
APP=$W/Payload/Sunpetal.app
VER=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[ -d generated/recomp ] || echo "предупреждение: нет generated/recomp, приложение будет без модулей игры (docs/BUILD.md)" >&2
export IPHONEOS_DEPLOYMENT_TARGET=15.0   # как MinimumOSVersion в Info.plist
cargo build --release --target aarch64-apple-ios -p sunpetal-app --bin sunpetal-app
rm -rf "$W" && mkdir -p "$APP"
cp target/aarch64-apple-ios/release/sunpetal-app "$APP/sunpetal-app"
cp LICENSE NOTICE "$APP/"
# значки iPhone: 120 и 180 px
python3 tools/android/make-icon.py --iconset "$W/iconset"
sips -z 120 120 "$W/iconset/icon_256x256.png" --out "$APP/Icon-120.png" >/dev/null
sips -z 180 180 "$W/iconset/icon_512x512.png" --out "$APP/Icon-180.png" >/dev/null
rm -rf "$W/iconset"
cat > "$APP/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Sunpetal</string>
  <key>CFBundleDisplayName</key><string>Sunpetal</string>
  <key>CFBundleIdentifier</key><string>org.sunpetal.app</string>
  <key>CFBundleExecutable</key><string>sunpetal-app</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VER</string>
  <key>CFBundleVersion</key><string>$VER</string>
  <key>CFBundleSupportedPlatforms</key><array><string>iPhoneOS</string></array>
  <key>MinimumOSVersion</key><string>15.0</string>
  <key>UIDeviceFamily</key><array><integer>1</integer><integer>2</integer></array>
  <key>UIRequiredDeviceCapabilities</key><array><string>arm64</string><string>metal</string></array>
  <key>UIRequiresFullScreen</key><true/>
  <key>UIStatusBarHidden</key><true/>
  <key>UIViewControllerBasedStatusBarAppearance</key><false/>
  <key>UISupportedInterfaceOrientations</key><array><string>UIInterfaceOrientationLandscapeLeft</string><string>UIInterfaceOrientationLandscapeRight</string><string>UIInterfaceOrientationPortrait</string></array>
  <key>UIFileSharingEnabled</key><true/>
  <key>LSSupportsOpeningDocumentsInPlace</key><true/>
  <key>CFBundleIcons</key><dict><key>CFBundlePrimaryIcon</key><dict><key>CFBundleIconFiles</key><array><string>Icon-120</string><string>Icon-180</string></array></dict></dict>
  <key>LSApplicationCategoryType</key><string>public.app-category.games</string>
</dict></plist>
EOF
plutil -lint "$APP/Info.plist" >/dev/null
rm -f "$OUT/Sunpetal.ipa"
(cd "$W" && zip -q -r -9 ../Sunpetal.ipa Payload)
unzip -tq "$OUT/Sunpetal.ipa"
ls -la "$OUT/Sunpetal.ipa"
