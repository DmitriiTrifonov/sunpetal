"""Тесты dsobjects.py:  python3 -m unittest tools/ps2/test_dsobjects.py -v"""
import os
import re
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import dsobjects

SAMPLE = os.path.join(HERE, 'testdata', 'gx_trace_sample.txt')


def _expected_vertices(lines):
    """Независимо (своими regex) считает сумму `n` по каждому ключу/tex образца."""
    keymap = {}   # (f, tex) -> [keys]
    batches = []  # (f, tex, n)
    for line in lines:
        s = line.strip()
        if not s:
            continue
        if s.startswith('K '):
            parts = s.split()
            f = int(parts[1].split('=', 1)[1])
            tex = int(parts[2].split('=', 1)[1], 0)
            key = parts[4].split('=', 1)[1]
            ft = (f, tex)
            keymap.setdefault(ft, [])
            if key not in keymap[ft]:
                keymap[ft].append(key)
        elif s.startswith('f='):
            f = int(re.search(r'f=(\d+)', s).group(1))
            n = int(re.search(r'n=(\d+)', s).group(1))
            tex = int(re.search(r'tex=(0x[0-9a-fA-F]+|\d+)', s).group(1), 0)
            batches.append((f, tex, n))
    out = {}
    for f, tex, n in batches:
        keys = keymap.get((f, tex))
        if keys:
            for k in keys:
                out[k] = out.get(k, 0) + n
        else:
            out[('tex', tex)] = out.get(('tex', tex), 0) + n
    return out


@unittest.skipUnless(os.path.exists(SAMPLE), 'нет образца трассы (получен из игры, не в git)')
class Sample(unittest.TestCase):
    def test_keys_attached_and_vertex_totals(self):
        with open(SAMPLE) as f:
            lines = f.readlines()
        catalog = dsobjects.build_catalog(dsobjects.parse_trace(lines))
        by_key = {e['key']: e for e in catalog}
        expected = _expected_vertices(lines)

        self.assertTrue(expected, 'в образце должны быть пачки с ключами')
        for key, want in expected.items():
            self.assertIsInstance(key, str, 'в образце все пачки имеют K-строку')
            self.assertIn(key, by_key, f'ключ {key} не подцеплен')
            self.assertEqual(by_key[key]['vertices'], want, f'вершины по ключу {key}')

        # Ключи без пачек есть в каталоге с нулём вершин.
        for key, obj in by_key.items():
            if key not in expected:
                self.assertEqual(obj['vertices'], 0, f'лишний ключ {key} должен быть пустым')

    def test_no_unknown_tex_in_sample(self):
        with open(SAMPLE) as f:
            catalog = dsobjects.build_catalog(dsobjects.parse_trace(f))
        self.assertFalse(any(e['key_unknown'] for e in catalog))
        self.assertFalse(any(e['ambiguous'] for e in catalog))


def _batch(f, b, tex, n, pos=None, mn=(-4096, -4096, -4096), mx=(4096, 4096, 4096)):
    if pos is None:
        pos = [4096, 0, 0, 0, 0, 4096, 0, 0, 0, 0, 4096, 0, 0, 0, 0, 4096]
    return (f'f={f} vb=0 b={b} prim=0 n={n} '
            f'min=[{mn[0]}, {mn[1]}, {mn[2]}] max=[{mx[0]}, {mx[1]}, {mx[2]}] '
            f'tex={tex} pos=[{", ".join(map(str, pos))}]')


def _key(f, tex, key, pal=0):
    return f'K f={f} tex={tex} pal=0x{pal:x} key={key}'


class Synthetic(unittest.TestCase):
    def _trace(self):
        lines = []
        # car-like: 4 пачки, одна матрица, 1000 вершин.
        for b in range(4):
            lines.append(_batch(1000, b, '0x0000000a', 250))
        lines.append(_key(1000, '0x0000000a', 'car'))
        # character-like: 10 пачек с 10 различными матрицами.
        for b in range(10):
            pos = [4096, 0, 0, 0, 0, 4096, 0, 0, 0, 0, 4096, 0, b, 0, 0, 4096]
            lines.append(_batch(1001, b, '0x0000000b', 10, pos=pos))
        lines.append(_key(1001, '0x0000000b', 'char'))
        # two keys one tex -> ambiguous.
        lines.append(_batch(1002, 0, '0x0000000c', 5))
        lines.append(_key(1002, '0x0000000c', 'amb1'))
        lines.append(_key(1002, '0x0000000c', 'amb2'))
        # other: 2 матрицы, 100 вершин.
        for b in range(2):
            pos = [4096, 0, 0, 0, 0, 4096, 0, 0, 0, 0, 4096, 0, b, 0, 0, 4096]
            lines.append(_batch(1004, b, '0x0000000e', 50, pos=pos))
        lines.append(_key(1004, '0x0000000e', 'other'))
        # оборванная последняя строка — пропускается.
        lines.append('f=1003 vb=0 b=0 prim=0 n=12 min=[-4096, -4096, -4096] '
                     'max=[4096, 4096, 4096] tex=0x0000000d pos=[4096, 0')
        return lines

    def test_classes_and_ambiguity(self):
        lines = self._trace()
        catalog = dsobjects.build_catalog(dsobjects.parse_trace(lines))
        by_key = {e['key']: e for e in catalog}

        self.assertEqual(by_key['car']['class'], 'car-like')
        self.assertEqual(by_key['car']['vertices'], 1000)
        self.assertEqual(by_key['car']['matrices'], 1)

        self.assertEqual(by_key['char']['class'], 'character-like')
        self.assertEqual(by_key['char']['matrices'], 10)

        self.assertEqual(by_key['other']['class'], 'other')

        self.assertTrue(by_key['amb1']['ambiguous'])
        self.assertTrue(by_key['amb2']['ambiguous'])
        self.assertEqual(by_key['amb1']['vertices'], 5)

    def test_truncated_last_line_is_skipped(self):
        records = list(dsobjects.parse_trace(self._trace()))
        frames = {r['f'] for r in records if r['type'] == 'batch'}
        self.assertNotIn(1003, frames)


if __name__ == '__main__':
    unittest.main()
