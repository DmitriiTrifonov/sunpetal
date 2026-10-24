#!/bin/sh
# Builds build/android/sunpetal.apk (arm64, signed with a debug key made on the first run).
# Needs: the Android SDK (platform 26+, build-tools), an NDK (r27), `cargo-ndk`, the Rust target
# aarch64-linux-android. ANDROID_HOME and ANDROID_NDK_HOME may be set; the defaults are the
# Android Studio folders on macOS.
set -e
cd "$(dirname "$0")/../.."
SDK=${ANDROID_HOME:-$HOME/Library/Android/sdk}
export ANDROID_NDK_HOME=${ANDROID_NDK_HOME:-$(ls -d "$SDK"/ndk/27.* | tail -1)}
BT=$(ls -d "$SDK"/build-tools/* | sort -V | tail -1)
JAR=$(ls -d "$SDK"/platforms/android-* | sort -V | tail -1)/android.jar
OUT=build/android
mkdir -p "$OUT/apk/lib/arm64-v8a"
cargo ndk -t arm64-v8a -P 26 build --release -p sunpetal-app --lib
cp target/aarch64-linux-android/release/libsunpetal_app.so "$OUT/apk/lib/arm64-v8a/"
# the picker activity (Java) to classes.dex
rm -rf "$OUT/classes" && mkdir -p "$OUT/classes"
javac -source 8 -target 8 -Xlint:-options -cp "$JAR" -d "$OUT/classes" tools/android/java/org/sunpetal/app/*.java
"$BT/d8" --min-api 26 --lib "$JAR" --output "$OUT/apk" $(find "$OUT/classes" -name '*.class')
rm -rf "$OUT/res" && python3 tools/android/make-icon.py "$OUT/res"
"$BT/aapt2" compile --dir "$OUT/res" -o "$OUT/res.zip"
"$BT/aapt2" link -o "$OUT/base.apk" -I "$JAR" --manifest tools/android/AndroidManifest.xml -R "$OUT/res.zip" \
    --min-sdk-version 26 --target-sdk-version 34 --version-code 1 --version-name 0.0.1
(cd "$OUT/apk" && zip -q -r -9 ../base.apk lib && zip -q ../base.apk classes.dex)
"$BT/zipalign" -f -p 4 "$OUT/base.apk" "$OUT/aligned.apk"
[ -f "$OUT/debug.keystore" ] || keytool -genkeypair -keystore "$OUT/debug.keystore" -storepass android -keypass android \
    -alias debug -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=Sunpetal debug" >/dev/null 2>&1
"$BT/apksigner" sign --ks "$OUT/debug.keystore" --ks-pass pass:android --key-pass pass:android \
    --out "$OUT/sunpetal.apk" "$OUT/aligned.apk"
rm -f "$OUT/base.apk" "$OUT/aligned.apk"
ls -la "$OUT/sunpetal.apk"
