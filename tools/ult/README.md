# tools/ult — импорт из PS2 (docs/ULTIMATE-IMPORT.md)

Окружение: `python3 -m venv build/venv && build/venv/bin/pip install numpy` (venv в `build/`, не в git). Большинству
скриптов хватает системного `python3`; `numpy` нужен там, где он импортируется.

```
EX=~/Developer/games/sunpetal-ee/out/export/v1
python3 tools/ult/check_export.py $EX [--fast]                       # приёмка выгрузки против контракта (~2 с, --fast без SHA-1)
python3 tools/ult/test_check_export.py                               # тесты приёмки на синтетической выгрузке
python3 tools/ult/make_scenario.py $EX ~/Developer/games/fsar-ru/text/ru.csv   # data/ult/scenario.toml
```

Дальше (совмещение карт, сборка областей, прогон сцен) — команды в `docs/NEXT.md` и журнале `docs/ULTIMATE-IMPORT.md`.
