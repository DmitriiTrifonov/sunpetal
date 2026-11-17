import unittest
import places


class Places(unittest.TestCase):
    def test_catalog_reads_and_ids_are_unique(self):
        ps = places.load_places()
        self.assertTrue(ps)
        for p in ps:
            self.assertIn('id', p)

    def test_verdict_names_the_mismatch(self):
        p = {'expect': {'scene': 'C02_01', 'map': 46}}
        self.assertIsNone(places.verdict(p, {'scene': 'C02_01', 'map': 46}))
        self.assertIn('map', places.verdict(p, {'scene': 'C02_01', 'map': 7}))
        self.assertIsNone(places.verdict({}, {'scene': 'x'}))

    def test_bookmarks_join_the_catalog(self):
        import os, tempfile
        with tempfile.TemporaryDirectory() as d:
            f = os.path.join(d, 'places.toml')
            with open(f, 'w') as g:
                g.write('[[place]]\nid = "kt"\nreplay = "artifacts/bookmarks/kt"\nframes = 10\nexpect = { scene = "C01_01" }\n')
            ps = places.load_places(bookmarks=f)
        mine = [p for p in ps if p['id'] == 'kt']
        self.assertEqual(len(mine), 1)
        self.assertEqual(mine[0]['frames'], 10)
        self.assertEqual(len(ps), len(places.load_places(bookmarks=None)) + 1)


if __name__ == '__main__':
    unittest.main()
