# AGENTS.md — Sunpetal

Статический рекомп DS-версии Flower, Sun, and Rain (Europe, `YHNP`) в нативное приложение. Рантайм на Rust, лицензия Apache-2.0. Документы на русском, коммиты и код: комментарии коротко, по делу.

Заметки проекта дублируются в Obsidian-vault: `~/Documents/synced-vault/synced-vault/03 Проекты/Sunpetal (рекомп)/` (там же журнал и инструменты). Правила vault — в его `AGENTS.md`.

## Прочитать первым делом

1. `docs/PLAN.md`: этапы 0–12, что сделано (`[x]`).
2. `docs/RECON-NDSKIT.md`: результаты разведки на нашем дампе.
3. `docs/PRIOR-ART.md`: существующие рекомпы DS, лицензии, что можно брать.

## Решения (не пересматривать без причины)

- База: **DS-версия**. PS2 только источник текстур (этап 7). Рекомпа PS2 нет.
- Рантайм и рекомпилятор: **Rust**. Сгенерированный код: сначала **C** (крейт `cc`), потом модули можно переводить на Rust.
- Лицензия: **Apache-2.0**. Код из GPL-проектов (melonDS, dust, legobattles-pc) **не копировать**, только читать. `ndskit` (MIT) брать можно, копирайт сохранять. `ndsrecomp` (PolyForm Noncommercial) только читать.
- Название **Sunpetal**. Не использовать название игры, персонажей, студии, логотипы.
- Ром, образы и всё, полученное из них (извлечённое, сгенерированный код), в репозиторий не коммитить (`.gitignore`).
- Моды (этап 12) позже, но архитектурно: рендерер получает два экрана отдельными текстурами, раскладка задаётся данными.

## Пути

| Что | Где |
|---|---|
| Репозиторий | `~/Developer/games/sunpetal` |
| DS-дамп (EUR, `YHNP`, SHA-1 `4922df25…`) | `~/Developer/games/fsar-ru/roms/Flower, Sun and Rain - Murder and Mystery in Paradise (Europe) (En,Fr,De,Es,It).nds` |
| PS2-образы (JP и английский патч v1.0) | `~/Developer/games/fsar-ru/roms/*.iso` |
| Проект перевода (скрипты разбора форматов, `text/ru.csv`) | `~/Developer/games/fsar-ru` |
| Чужие репо для чтения | `~/Developer/games/ref/ndskit` (MIT), `~/Developer/games/ref/ndsrecomp` (PolyForm NC) |
| Результаты прогона ndskit (не в git) | `build/ndskit/` (`recomp/` = сгенерированный C++, `arm9_vectors.txt`) |

## Как воспроизвести прогон ndskit

```sh
ROM="$HOME/Developer/games/fsar-ru/roms/Flower, Sun and Rain - Murder and Mystery in Paradise (Europe) (En,Fr,De,Es,It).nds"
export PYTHONPATH=$HOME/Developer/games/ref        # пакет ndskit лежит в ref/ndskit
python3 -m ndskit.rom "$ROM" --info
python3 -m ndskit.recomp.discover "$ROM" --cpu 9 --report --out build/ndskit/funcs9.tsv
python3 -m ndskit.recomp "$ROM" --out build/ndskit/recomp          # ~40 с
cmake -S build/ndskit/recomp -B build/ndskit/recomp/b2 -G Ninja -DCMAKE_CXX_COMPILER=clang++ \
  -DCMAKE_CXX_FLAGS="-D_XOPEN_SOURCE=600 -D_DARWIN_C_SOURCE"
ninja -C build/ndskit/recomp/b2 selftest
python3 -m ndskit.recomp.selftest "$ROM" --cpu 9 --auto --out build/ndskit/arm9_vectors.txt
build/ndskit/recomp/b2/selftest build/ndskit/arm9_vectors.txt     # 0 failures
```

Ограничения на macOS: `ndsrun` не собирается (нужен OpenGL 4.5). Это нормально: графику пишем сами.

## Состояние на 2026-10-24

Рантайм исполняет рекомпилированную игру безголово и в окне (`sunpetal-app`, с меню настроек): логотипы, титул, меню «SAVE DATA», вступительная сцена; 2D, 3D, звук; `cargo test` (12 тестов) зелёный. Полная картина, список недоделок и вопросов: `docs/STATUS.md`. Шов C ↔ Rust: `docs/ABI.md`.

Осталось в этапе 0: проверка названия по WIPO/EUIPO/USPTO (вручную), изучение `ndsrecomp` (только чтение). Хостинг: репозиторий на GitHub, домен не берём.

## Как воспроизвести прогон ndskit

```sh
ROM="$HOME/Developer/games/fsar-ru/roms/Flower, Sun and Rain - Murder and Mystery in Paradise (Europe) (En,Fr,De,Es,It).nds"
export PYTHONPATH=$HOME/Developer/games/ref        # пакет ndskit лежит в ref/ndskit
python3 -m ndskit.rom "$ROM" --info
python3 -m ndskit.recomp.discover "$ROM" --cpu 9 --report --out build/ndskit/funcs9.tsv
python3 -m ndskit.recomp "$ROM" --out build/ndskit/recomp          # ~40 с
cmake -S build/ndskit/recomp -B build/ndskit/recomp/b2 -G Ninja -DCMAKE_CXX_COMPILER=clang++ \
  -DCMAKE_CXX_FLAGS="-D_XOPEN_SOURCE=600 -D_DARWIN_C_SOURCE"
ninja -C build/ndskit/recomp/b2 selftest
python3 -m ndskit.recomp.selftest "$ROM" --cpu 9 --auto --out build/ndskit/arm9_vectors.txt
build/ndskit/recomp/b2/selftest build/ndskit/arm9_vectors.txt     # 0 failures
```

Ограничения на macOS: `ndsrun` не собирается (нужен OpenGL 4.5). Это нормально: графику пишем сами.

## Состояние на 2026-10-24

Этап 0 почти закрыт, шаг «шов C ↔ Rust» сделан, этап 2 закрыт (рекомпилятор, интерпретатор, оверлеи, самотест), этап 3 в основном сделан (ядро железа, BIOS, потоки; безголовый запуск идёт), этап 4 в основном сделан в программном виде (2D, 3D, PNG, скрипт ввода; игра доходит до меню и вступительной сцены), звук есть (`--wav`). Сделано: выбраны язык, лицензия, база, название; хостинг — просто репозиторий на GitHub, домен не берём; найдены аналоги; `ndskit` прогнан на дампе; Python-часть `ndskit` форкнута в `tools/ndskit/`, эмиттер пишет чистый C; Rust-каркас `crates/sunpetal-runtime` (Cpu, шина, режимы, диспетчер, оверлеи по crc, CP15-массив), `build.rs` собирает сгенерированный C через `cc`. `cargo test -p sunpetal-runtime`: 542 функции ARM9 игры (8537 векторов) и инструкционные тесты ARM9/ARM7, 0 расхождений. ABI в `docs/ABI.md`.

Осталось в этапе 0: проверка названия по WIPO/EUIPO/USPTO (вручную), изучение `ndsrecomp` (только чтение).

## Как воспроизвести прогон (наш форк)

```sh
ROM="$HOME/Developer/games/fsar-ru/roms/Flower, Sun and Rain - Murder and Mystery in Paradise (Europe) (En,Fr,De,Es,It).nds"
export PYTHONPATH=$PWD/tools                      # пакет ndskit лежит в tools/ndskit
python3 -m ndskit.recomp "$ROM" --out generated/recomp --optest          # ~45 с, C + векторы optest
python3 -m ndskit.recomp.selftest "$ROM" --cpu 9 --auto --out generated/recomp/selftest/arm9_vectors.txt
for o in 1 3 4 5 6; do python3 -m ndskit.recomp.selftest "$ROM" --cpu 9 --overlay $o --auto --out generated/recomp/selftest/arm9_ov${o}_vectors.txt; done
SUNPETAL_ROM="$ROM" cargo test -p sunpetal-runtime   # ~1,5 мин (первая сборка C)
cargo run --release -p sunpetal-runtime --bin sunpetal-headless -- "$ROM" --vblanks 600   # безголовый запуск
```

Без `generated/recomp` крейт собирается без модулей, тесты самотеста не компилируются. `generated/` в `.gitignore`.

## Следующий шаг (начать с него)

Сохранение игры закрыто (проверено руками 2026-10-26). Этап 7 (рантайм-часть, PS2-текстуры: покрытие 732 из 1615) и этап 8 (bloom, FXAA, контур, мягкие тени) в основном сделаны. Полный список открытого: `docs/PLAN.md`, состояние: `docs/STATUS.md`.

Решения 2026-10-26 (подробно в `docs/PLAN.md`, этапы 8б–9б, 10):

- Новые этапы: **8б** режим одного экрана, **8в** соотношение сторон и разрешение, **8г** меню в стиле игры, **9б** компоновщик и `sunpetal.data`.
- Игра берёт данные из `sunpetal.data` (архив, без кода; части: ROM DS обязательна, PS2, перевод, эффекты); запасной путь: подсунуть ROM DS. ROM DS обязателен только для сборки `sunpetal.data`.
- Код игры: приложение с нативным кодом собирает сам пользователь на ПК (вариант «а»); общее приложение с интерпретатором («б») оставлено на потом.
- iOS только сайдлоад/AltStore, в сторы не выкладываем. Репозиторий на GitHub уже есть (`DmitriiTrifonov/sunpetal`), CI зелёный.
- Перевод принадлежит автору проекта; в дистрибутив идёт только дельта, без текста оригинала.

Сделано в этапе 9 (2026-10-26, `docs/TRANSLATION-RECON.md`, `docs/STATUS.md`): крейт `crates/sunpetal-data` (nitrofs, lz10, сценарии, дельты `SPD1`, контейнер `.spdata`), слой замены файлов NitroFS, секция перевода `TRAN` (русский 680 КБ), `sunpetal-pack-translation`, ключи `--data/--lang/--overlay`, «Язык игры» в настройках. Вживую в окне русский проверен 2026-10-27.

Продолжать с: (1) (сделано: русский в окне проверен); (2) этап 9б: секция `META`, компоновщик (приложение, собирающее файл из ROM, образа PS2 и перевода), загрузчик и запасной путь с ROM; (3) этап 9, остаток: инструменты переводчиков, шрифты и перевод интерфейса приложения, документация «как сделать свой язык»; (4) 8б/8в (один экран, пропорции; вместе с раскладками для телефона из этапа 5) → 8г (меню) → 10 (платформы) → остаток этапа 7 (2D-фоны, `YOSHI`/`GDB`, модели `MODL`) и MSAA.

Недоделки: артефакты роликов (возможно, низкий битрейт), блочная дорога на мосту (фильтрация/перспектива текстур), звук на слух, геймпад, Android на устройстве.

## Правила работы

- Не писать в `~/Developer/games/fsar-ru` (там работающий проект перевода). Из него только читать.
- Чужой код из `ref/` не менять и не коммитить сюда.
- Новые решения и результаты исследований: в `docs/` репозитория и в заметки vault.
- Коммиты только по просьбе пользователя. В конце сообщения коммита: `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
