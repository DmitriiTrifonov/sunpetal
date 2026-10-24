# 🧰 Инструменты: Sunpetal

> Проект: `Sunpetal (рекомп)` · План: [План](PLAN.md)

Список из памяти, **проверить актуальность и работу на macOS**. Уже стоит: ndstool 2.3.1, xdelta 3.2.2, ndspy, melonDS (см. `Перевод Flower, Sun and Rain`).

## Разбор бинарников и ассетов

| Задача | Инструмент |
|---|---|
| Дизассемблер/декомпилятор ARM | Ghidra (бесплатно), IDA Free как запасной |
| Распаковка ROM | ndstool, Tinke, ndspy |
| Форматы DS (NCGR, NCLR, NSCR, NFTR) | свои скрипты из `fsar-ru/scripts` (`ds_gfx.py`, `nftr.py`) |
| Образ PS2 (ISO), извлечение | 7-Zip / `isoinfo` / PCSX2 (просмотр), потом свои скрипты под архивы игры |
| Текстуры PS2 (TIM2 и др.) | свой конвертер на Python, при необходимости Noesis (Windows/Wine) или Kaitai Struct для описания форматов |
| Бинарный анализ | ImHex, Kaitai Struct, `binwalk` |

## Эмуляторы и отладка (эталон поведения)

- **melonDS**: основной эталон, есть GDB-стаб и сейвстейты (использовали в `Перевод Flower, Sun and Rain`)
- **DeSmuME**: запасной, есть отладчик
- **no$gba** (Windows/Wine): отладчик с хорошими средствами
- **PCSX2**: только для извлечения и сверки PS2-ассетов
- Трассы регистров/памяти для сверки рекомпа с эталоном

## Рекомпилятор и рантайм

- Язык рантайма: **Rust** (stable, cargo); рекомпилятор тоже на Rust (или на Python для прототипа, как `ndskit`)
- Генерируемый код: C через крейт `cc` или Rust (решить на этапе 0)
- Декодер ARM/Thumb: свой; `capstone` (крейт) для дизассемблирования при отладке
- Образцы: `ndsrecomp`, `ndskit` (для DS), N64Recomp, Zelda64Recomp, RT64 (архитектура, замена текстур, UI); подробности в [Существующие рекомпы DS](PRIOR-ART.md)
- Крейты (кандидаты): `sdl3` или `winit` + `wgpu`, `cpal`, `serde` + `toml`, `egui`, `image`, `clap`, `tracing`
- Мобильные сборки Rust: `cargo-ndk` (Android), `cargo-mobile2` или Xcode-проект со статической библиотекой (iOS), `uniffi`/`cbindgen` при необходимости
- Тесты: `cargo test`, `criterion` для бенчмарков
- Конфиги: TOML (toml++ / `toml` crate)
- Логи и тесты: spdlog, GoogleTest / Catch2, `cargo test`

## Графика

- **SDL3**: окно, ввод, звук, тач, геймпады на всех платформах
- Рендер: **wgpu** (Metal на macOS/iOS, Vulkan на Android/Linux, D3D12 на Windows)
- **librashader**: запуск шейдеров формата RetroArch (slang) для эффектов
- Шейдеры: glslang / SPIRV-Cross / Slang, Metal Shader Converter (Apple)
- Апскейл текстур: Real-ESRGAN / waifu2x, ImageMagick для пакетной обработки
- Форматы текстур: PNG/DDS (BC7, ASTC для мобильных), `astcenc`, `texconv`
- Отладка GPU: RenderDoc (Linux/Windows), Xcode GPU Frame Capture (macOS/iOS)

## Звук

- SDL3 audio, при необходимости miniaudio
- Разбор SDAT (звук DS): свои скрипты, vgmstream для проверки

## Интерфейс и настройки

- Dear ImGui (быстро) или RmlUi (стилизуемо, как в Zelda64Recomp)
- Тач-оверлей для мобильных: свой слой поверх SDL
- Хранение настроек: TOML/JSON в папке приложения

## Перевод и тексты

- Таблица `ru.csv` из `Перевод Flower, Sun and Rain` как исходник языкового пакета
- Шрифты: растровые NFTR (как сейчас) или FreeType/stb_truetype для TTF
- Проверка ширины строк: `check_table.py`, `linewidth.py`
- Патчи-дельты: xdelta для варианта «патч для ROM», свой формат пакета для рантайма

## Платформы и сборка

- **macOS**: Xcode + Command Line Tools, CMake, Homebrew
- **Linux**: gcc/clang, CMake, Ninja, AppImage (linuxdeploy)
- **Windows**: Visual Studio Build Tools или clang-cl, NSIS/Inno Setup
- **Android**: Android Studio, NDK, Gradle, `adb`
- **iOS**: Xcode, Apple Developer-аккаунт (сайдлоад: AltStore/Sideloadly), нужен Mac
- **CI**: GitHub Actions (матрица macOS/Linux/Windows/Android/iOS), кэш ccache
- **Git** + Git LFS (только для своих ассетов, не оригинальных)

## Вспомогательное

- Python 3.12 + `venv`, `ndspy`, `Pillow`, `numpy` для скриптов
- Хранилище ассетов вне хранилища Obsidian: `~/Developer/games/sunpetal` (ром и оригинальные данные в git не попадают)
- Проверка названия: USPTO TESS, EUIPO eSearch, WIPO Global Brand Database, поиск по GitHub и сторам
