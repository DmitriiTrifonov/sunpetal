#!/usr/bin/env python3
"""Writes docs/THIRD-PARTY.md: the Rust crates the build uses, with their licences.

    tools/third-party.py        (needs `cargo` and the network the first time)
"""
import json
import subprocess
import sys

meta = json.loads(subprocess.check_output(['cargo', 'metadata', '--format-version', '1', '--locked'], text=True))
ours = {m for m in meta['workspace_members']}
rows = sorted(
    (p['name'], p['version'], p.get('license') or 'see the crate', p.get('repository') or '')
    for p in meta['packages'] if p['id'] not in ours and p['source'] is not None
)
out = ['# Сторонние компоненты', '',
       'Крейты Rust, из которых собирается Sunpetal (по `Cargo.lock`), и их лицензии. Список создаёт',
       '`tools/third-party.py`, вручную не править.', '',
       f'Всего: {len(rows)}.', '', '| Крейт | Версия | Лицензия | Репозиторий |', '|---|---|---|---|']
out += [f'| {n} | {v} | {l} | {r} |' for n, v, l, r in rows]
out += ['', '## Не из Cargo', '',
        '- `tools/ndskit`: MIT, © 2026 Samuele Voltan (форк Python-части ndskit, см. `tools/ndskit/LICENSE` и `UPSTREAM.md`).',
        '- Шрифты чёткого текста реплик (встроены в приложение, `crates/sunpetal-app/fonts/`): Noto Sans (© The Noto Project Authors, SIL OFL 1.1, `NotoSans-OFL.txt`) и Rubik (© The Rubik Project Authors, SIL OFL 1.1, `Rubik-OFL.txt`). Оба вариативные, вес 400 задаётся в коде; скачаны 2026-10-30 из `github.com/google/fonts` (`ofl/notosans`, `ofl/rubik`), файлы не менялись.',
        '- Fira Sans Extra Condensed Medium (© 2012–2015 The Mozilla Foundation and Telefonica S.A., SIL OFL 1.1): шрифт не встроен и не раздаётся; им `tools/ps2/menu_icons_pack.py` и `case_up.py` рисуют подписи в картинках пакета 2D (`video.pack_2d`). OFL разрешает такие картинки без условий на их лицензию; сам TTF лежит только в `build/fonts` (не в git), текст лицензии — `build/fonts/firasansextracondensed/OFL.txt`.',
        '- Шрифт меню: Ubuntu Font (встроен в `egui` через `epaint_default_fonts`; лицензия Ubuntu Font Licence 1.0).',
        '- Слабый копилефт: `option-ext` (MPL-2.0, через `dirs`), используется без изменений. Двойные лицензии с GPL/LGPL (`self_cell`, `r-efi`) берутся по варианту Apache-2.0/MIT.',
        '- Код из GPL-проектов (melonDS и др.) в Sunpetal не копировался.', '']
open('docs/THIRD-PARTY.md', 'w').write('\n'.join(out))
print(len(rows), 'крейтов', file=sys.stderr)
