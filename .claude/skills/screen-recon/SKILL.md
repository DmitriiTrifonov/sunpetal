---
name: screen-recon
description: Разведка экранов игры без окна через sunpetal-debug (REPL): маршрут до свободной ходьбы, чемодана, устройства Катарины, титула и Request, снимки обоих экранов, чтение регистров и OAM. Использовать, когда нужно снять кадры сцен/меню, не трогая окно пользователя.
---

# Разведка экранов через REPL

Нужно сохранение с открытыми запросами: `--save build/full100.sav` (другие сохранения: `docs/debug-tools.md`). Снимки класть в `artifacts/screens/` (не в git).

```sh
printf '%s\n' "skip 100" "advance 20" "press START 3" "advance 40" "tap 25 43 4" "advance 60" "shot artifacts/screens/x.png" "quit" \
 | target/release/sunpetal-debug "$ROM" --save build/full100.sav --goto C02_01 --autoplay --scale 1 --repl
```

Маршруты:
- Свободная ходьба: `--goto C02_01 --autoplay` (годятся и `C04_01`, `C05_01`); `skip` отвечает `"stopped":"idle"`. Из сцен с диалогом `skip` не выходит.
- Меню ходьбы: `press START 3`, `advance 40` → значки слева на нижнем экране (чемодан ≈ x25,y43; значок «018» — счётчик шагов, не кнопка).
- Чемодан: `tap 25 43 4` → экран «CATHERINE»; `tap 128 60 4` → устройство; `tap 128 20 4` → пункты чемодана (y≈90: GUIDE BOOK x≈50, MEMO ≈100, SAVE ≈150, GOODS ≈203).
- Титул: без `--goto`, `advance 1500`, `press START 3`, `advance 120`; Request ≈ (190, 80), кнопки R-00/R-01/R-02 видны около 15 кадров после касания.
- Регистры и OAM нижнего экрана: `peek 04001000 4` (DISPCNT B), `peek 04001008 8` (BGCNT), `peek 07000400 1024` (OAM B); верхний — `04000000`, `07000000`.

Подводные камни: касания даются в координатах нижнего экрана (256×192); `shotwide` не работает (`wide:false`); часы логотипов в окне и в REPL не совпадают (расписание `layout::logo_screen` сверять по снимкам окна, не по REPL); нет PIL — картинки смотреть по одной через Read.
