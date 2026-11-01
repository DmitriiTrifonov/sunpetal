---
name: window-restart
description: Запуск и перезапуск окна игры (sunpetal-app) для пользователя. Использовать, когда пользователь просит запустить игру или говорит «перезапуск/перезапусти», и при проверке, что окно одно.
---

# Окно игры пользователя

В окне может быть несохранённая игра, поэтому:

- Без слова пользователя «перезапуск/перезапусти» окно не трогать и не гасить (`pkill` запрещён).
- Просьба «запусти» без окна: сначала `pgrep -fl sunpetal-app`; если окно уже есть, сказать об этом и не запускать второе.
- «Перезапуск»: `kill` по pid из `pgrep` (не `pkill`), пауза 2 с, снова `pgrep` (чтобы не остались два окна), затем запуск отдельным процессом без привязки к сессии:

```sh
ROM="$HOME/Developer/games/fsar-ru/roms/Flower, Sun and Rain - Murder and Mystery in Paradise (Europe) (En,Fr,De,Es,It).nds"
SUNPETAL_RAM_DUMP=1 nohup target/release/sunpetal-app "$ROM" --lang ru > build/vtext-app.log 2>&1 < /dev/null & disown
```

- Перед перезапуском пересобрать: `cargo build --release -p sunpetal-app`.
- В конце сообщить новый pid и что посмотреть. Снимки по F10 лежат в `artifacts/shots/` (`window_N.png` — окно, `vN.png` — оба экрана DS, `vN.ram` — ОЗУ при `SUNPETAL_RAM_DUMP=1`).
