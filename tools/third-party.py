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
        '- Шрифт меню: Ubuntu Font (встроен в `egui` через `epaint_default_fonts`; лицензия Ubuntu Font Licence 1.0).',
        '- Слабый копилефт: `option-ext` (MPL-2.0, через `dirs`), используется без изменений. Двойные лицензии с GPL/LGPL (`self_cell`, `r-efi`) берутся по варианту Apache-2.0/MIT.',
        '- Код из GPL-проектов (melonDS и др.) в Sunpetal не копировался.', '']
open('docs/THIRD-PARTY.md', 'w').write('\n'.join(out))
print(len(rows), 'крейтов', file=sys.stderr)
