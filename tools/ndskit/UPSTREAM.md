# Происхождение

Python-часть [ndskit](https://github.com/vs-sr-dev/ndskit) (автор Samuele Voltan, MIT, см. `LICENSE` рядом).
Копия из локального клона `~/Developer/games/ref/ndskit`, коммит
`5ee24aff2ed0dfe8359fc0ade4477a25e4eb954f` (2026-10-11, «ndskit: the README with its third game»).

Взято: `rom.py`, `arm.py`, `armemu.py`, `lz.py`, `sdat.py`, `hw.py`, `recomp/` (`discover`, `emit`, `selftest`, `__main__`).
Не взято: `runtime/` (C++, читаем как образец), `melonds.py`, `ghidra/`.

Дальше файлы правятся здесь (в первую очередь `recomp/emit.py`: вывод на C с ABI из `docs/ABI.md`).
Изменения относительно upstream — в истории git этого репозитория. Копирайт и текст лицензии сохранены.
Запуск: `PYTHONPATH=tools python3 -m ndskit.rom ...`
