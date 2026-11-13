#!/usr/bin/env python3
"""Тесты check_export.py на синтетической выгрузке (У0.2): python3 tools/ult/test_check_export.py"""
import hashlib
import json
import os
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import check_export as ce


def put(root, rel, data):
    p = os.path.join(root, rel)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    b = data if isinstance(data, bytes) else data.encode()
    open(p, 'wb').write(b)
    return {'path': rel, 'sha1': hashlib.sha1(b).hexdigest(), 'size': len(b)}


def build(root):
    files = []
    tex = 'aaaaaaaaaa_bbbbbbbb'
    files.append(put(root, f'textures/{tex}.png', b'png'))
    files.append(put(root, 'textures.tsv', f'sha\tw\n{tex}\t8\n'))
    mid = 'cccccccccc'
    model = {'id': mid, 'sha1': mid + 'd' * 30, 'counts': {'vertices': 3},
             'materials': [{'texture': tex}, {'texture': None}],
             'series': [{'material': 0, 'first_vertex': 0, 'count': 3}]}
    files.append(put(root, f'models/{mid}.json', json.dumps(model)))
    files.append(put(root, f'models/{mid}.glb', b'glb'))
    files.append(put(root, f'models/{mid}.vtx.bin', b'\0' * 120))
    files.append(put(root, 'motions/eeeeeeeeee.json', '{}'))
    scene = {'scene3d': [{'models': {'GIG': mid}}], 'chara': [], 'motion_pack': [{'motion': 'eeeeeeeeee'}]}
    files.append(put(root, 'scenes/R00.json', json.dumps(scene)))
    files.append(put(root, 'events/R00.jsonl', '{"a":1}\n{"b":2}\n'))
    man = {'schema': 1, 'date': 'x', 'files': files,
           'counts': {'models_in_scenes': 1, 'models_extra': 0, 'motions': 1}}
    json.dump(man, open(os.path.join(root, 'manifest.json'), 'w'))


class T(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp()
        build(self.d)

    def tearDown(self):
        shutil.rmtree(self.d)

    def run_check(self, **kw):
        return ce.check(self.d, expect_scenes=1, **kw)

    def test_clean(self):
        e, w, _ = self.run_check()
        self.assertEqual((e, w), ([], []))

    def test_sha_changed(self):
        open(os.path.join(self.d, 'models/cccccccccc.glb'), 'wb').write(b'xyz')
        e, _, _ = self.run_check()
        self.assertTrue(any('SHA-1' in x for x in e))
        e, _, _ = self.run_check(fast=True)
        self.assertEqual(e, [])

    def test_missing_file(self):
        os.remove(os.path.join(self.d, 'motions/eeeeeeeeee.json'))
        e, _, _ = self.run_check()
        self.assertTrue(any('файла нет' in x for x in e))
        self.assertTrue(any('движений 0' in x for x in e))

    def test_vtx_size_and_series(self):
        p = os.path.join(self.d, 'models/cccccccccc.json')
        m = json.load(open(p))
        m['counts']['vertices'] = 2
        json.dump(m, open(p, 'w'))
        e, _, _ = self.run_check(fast=True)
        self.assertTrue(any('vtx.bin' in x for x in e))
        self.assertTrue(any('серии доходят' in x for x in e))

    def test_dangling_refs(self):
        p = os.path.join(self.d, 'scenes/R00.json')
        s = json.load(open(p))
        s['scene3d'][0]['models']['GIG4'] = 'ffffffffff'
        s['motion_pack'][0]['motion'] = '0000000000'
        json.dump(s, open(p, 'w'))
        e, _, _ = self.run_check(fast=True)
        self.assertTrue(any('ffffffffff' in x for x in e))
        self.assertTrue(any('0000000000' in x for x in e))

    def test_extra_file_warns(self):
        put(self.d, 'audio/new.bin', b'1')
        e, w, _ = self.run_check()
        self.assertEqual(e, [])
        self.assertEqual(len(w), 1)

    def test_bad_jsonl_and_texture(self):
        put(self.d, 'events/R00.jsonl', '{"a":1}\nnot json\n')
        open(os.path.join(self.d, 'textures.tsv'), 'a').write('zzzzzzzzzz_yyyyyyyy\t8\n')
        e, _, _ = self.run_check(fast=True)
        self.assertTrue(any('R00.jsonl:2' in x for x in e))
        self.assertTrue(any('zzzzzzzzzz' in x for x in e))


if __name__ == '__main__':
    unittest.main()
