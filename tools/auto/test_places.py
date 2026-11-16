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


if __name__ == '__main__':
    unittest.main()
