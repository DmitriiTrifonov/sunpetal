# Android

Состояние на 2026-10-24: собирается `apk` (arm64), **на устройстве не запускался** (телефона под рукой не было, эмулятора с подходящим образом нет). Нужно проверить: запускается ли, показывает ли картинку, идёт ли звук (AAudio), хватает ли скорости.

## Сборка

Нужны: Android SDK (платформа 26+, build-tools), NDK r27, `cargo install cargo-ndk`, `rustup target add aarch64-linux-android`.

```sh
tools/android/build-apk.sh        # build/android/sunpetal.apk, подпись отладочным ключом
adb install -r build/android/sunpetal.apk
adb push rom.nds /sdcard/Android/data/org.sunpetal.app/files/rom.nds   # ром (папка появляется после первого запуска)
```

Скрипт: `cargo ndk` собирает `libsunpetal_app.so` (приложение как `cdylib`, точка входа `android_main`, `NativeActivity` из `winit`), `aapt2` пакует манифест (`tools/android/AndroidManifest.xml`), `zipalign`, `apksigner`. Сгенерированный C собирается clang из NDK через `cc` в `build.rs`.

Логи (всё, что приложение пишет в stderr): `adb logcat -s sunpetal`.

## Как устроено

- ROM: `rom.nds` в `Android/data/org.sunpetal.app/files/` (общий накопитель, достаётся через `adb push` и файловый менеджер на Android до 11). Сохранения кладутся рядом; настройки в приватной папке приложения.
- Картинка: те же `wgpu` и `egui`, что на десктопе. Звук: `cpal` (AAudio). Касание = стилус на нижнем экране.
- Внутреннее разрешение 3D по умолчанию 1× (рендер программный и однопоточный).

## Не сделано

- Кнопки на экране (крестовина, A/B/X/Y, L/R, Start/Select) и раскладка для телефона. Без них играть нечем (только геймпад по Bluetooth, но `gilrs` на Android не работает).
- Выбор ROM через системный диалог (SAF): нужен Java-класс-наследник `NativeActivity` и компиляция в dex.
- Сообщение на экране, если ROM не найден (сейчас приложение просто закрывается, причина в logcat).
- Меню настроек под палец, ориентация, фоновые режимы, иконка, подпись для релиза, armeabi-v7a и x86_64.
