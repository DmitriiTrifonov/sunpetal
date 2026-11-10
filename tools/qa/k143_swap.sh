#!/bin/sh
# K-143, шаг 4: горячая подмена картинок пакета 2D при открытом окне, сцена C01_01.
# on  — в пакет build/2d-pack-case кладутся три картинки сцены (фон нижнего экрана, фон-слой 1, звезда 64x64),
#       окрашенные в красный; исходники — build/k143-src (выгрузка F10 из C01_01);
# off — эти три файла убираются из пакета (в пакете их раньше не было).
# Окно должно подхватить это само: «2D pack reloaded» в build/qa-app.log.
cd "$(dirname "$0")/../.." || exit 1
P=build/2d-pack-case; S=build/k143-src
case "$1" in
  on)
    for f in "$S"/*.png; do magick "$f" -fill red -colorize 55% "$P/$(basename "$f")"; done
    echo "в пакете красных: $(ls "$S" | wc -l)" ;;
  off)
    for f in "$S"/*.png; do rm -f "$P/$(basename "$f")"; done
    echo "убрано: $(ls "$S" | wc -l)" ;;
  *) echo "usage: $0 on|off"; exit 1 ;;
esac
