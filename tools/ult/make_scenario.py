#!/usr/bin/env python3
"""Таблица сценариев: глава DS ↔ ScenarioID ↔ файл PS2 (У0.4) → data/ult/scenario.toml.

    make_scenario.py EXPORT_DIR RU_CSV [OUT.toml]

ScenarioID → файл: `SIDtoRID` (sunpetal-ee, notes/findings/f6-assets.md F-116) = [0,1..7,10,8,9,11,12,13,15..19],
0 = `OP`; New Game = ScenarioID 20 = `R00`. Глава DS `Cnn` = ScenarioID nn (C00 = OP). Число реплик: DS —
строки `text/ru.csv` проекта перевода (только читается), PS2 — `text/<R>.tsv` выгрузки (колонка sentences).
`ok` = расхождение не больше 10 %.
"""
import collections
import csv
import os
import sys

SID_TO_RID = [0, 1, 2, 3, 4, 5, 6, 7, 10, 8, 9, 11, 12, 13, 15, 16, 17, 18, 19]
NEW_GAME_SID = 20


def rname(n):
    return 'OP' if n == 0 else 'R%02d' % n


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    export, ru = argv[0], argv[1]
    out = argv[2] if len(argv) > 2 else os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', 'data', 'ult', 'scenario.toml')
    ds = collections.Counter(r['Глава'] for r in csv.DictReader(open(ru, encoding='utf-8')))

    def ps2(name):
        p = os.path.join(export, 'text', name + '.tsv')
        if not os.path.isfile(p):
            return None
        return sum(int(r['sentences']) for r in csv.DictReader(open(p, encoding='utf-8'), delimiter='\t'))

    rows = [(sid, 'C%02d' % sid, rname(r)) for sid, r in enumerate(SID_TO_RID)]
    rows.append((NEW_GAME_SID, None, 'R00'))
    lines = ['# Глава DS ↔ ScenarioID ↔ файл PS2 (tools/ult/make_scenario.py; руками не править).',
             '# ds_lines: реплик главы в text/ru.csv; ps2_lines: sentences в text/<R>.tsv выгрузки; diff = ps2/ds - 1;',
             '# ok: |diff| <= 0,10. В DS реплик больше в большинстве глав: проверка ключом «число реплик» слабая, порядок',
             '# глав взят из SIDtoRID (он описывает счёт лучше прямой нумерации: разброс 0,14 против 0,17).', '']
    bad = 0
    for sid, c, r in rows:
        d = ds.get(c) if c else None
        p = ps2(r)
        lines += ['[[scenario]]', f'scenario_id = {sid}', f'ps2 = "{r}"']
        if c:
            lines.append(f'ds = "{c}"')
        if d is not None and p is not None:
            diff = p / d - 1
            ok = abs(diff) <= 0.10
            bad += not ok
            lines += [f'ds_lines = {d}', f'ps2_lines = {p}', f'diff = {diff:.3f}', f'ok = {str(ok).lower()}']
        else:
            lines.append('note = "новая игра: пара в DS не определена (нужна проверка по сценам)"' if r == 'R00' else 'note = "нет данных"')
        lines.append('')
    open(out, 'w', encoding='utf-8').write('\n'.join(lines))
    print(f'{len(rows)} строк, {bad} вне 10 % → {out}')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
