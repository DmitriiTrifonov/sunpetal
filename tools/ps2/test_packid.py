import csv, os, sys, tempfile, unittest
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import packid

A = 'obj0_8x8_%016x_%016x' % (0xaa, 0xbb)
B = 'obj0_8x8_%016x_%016x' % (0xcc, 0xdd)
C = 'obj1_8x8_%016x_%016x@digits.rubik' % (1, 2)


class T(unittest.TestCase):
    def mk(self):
        d = tempfile.mkdtemp()
        for k, b in ((A, b'same'), (B, b'same'), (C, b'other')):
            open(os.path.join(d, k + '.png'), 'wb').write(b)
        open(os.path.join(d, 'index.csv'), 'w').write('x')
        return d

    def test_fold_merges_equal_pictures_and_names_by_csv(self):
        d = self.mk()
        ids = packid.fold(d, {A: 'push'})
        self.assertEqual(sorted(ids), sorted(['push', 'p_' + __import__('hashlib').sha1(b'other').hexdigest()[:8]]))
        self.assertEqual(sorted(ids['push']), sorted([A, B]))
        self.assertTrue(os.path.exists(os.path.join(d, 'push.png')))
        self.assertFalse(os.path.exists(os.path.join(d, A + '.png')))
        m = packid.read_manifest(open(os.path.join(d, 'pack.toml')).read())
        self.assertEqual({k: sorted(v) for k, v in m.items()}, {k: sorted(v) for k, v in ids.items()})
        self.assertEqual(open(os.path.join(d, 'index.csv')).read(), 'x')

    def test_two_ids_for_one_picture_is_refused(self):
        with self.assertRaises(SystemExit):
            packid.fold(self.mk(), {A: 'a', B: 'b'})

    def test_index_gets_an_id_column(self):
        d = self.mk()
        packid.fold(d, {A: 'push'})
        dump = tempfile.mkdtemp()
        with open(os.path.join(dump, 'index.csv'), 'w') as f:
            f.write('kind,engine,layer_or_oam,x,y,w,h,key,file\nobj,0,1,0,0,8,8,k,%s.png\nobj,0,2,0,0,8,8,k,nope.png\n' % B)
        self.assertEqual(packid.add_index(dump, d), 1)
        self.assertEqual(packid.add_index(dump, d), 1, 'again: the column is replaced')
        rows = list(csv.reader(open(os.path.join(dump, 'index.csv'))))
        self.assertEqual([r[9] for r in rows], ['id', 'push', ''])


unittest.main() if __name__ == '__main__' else None
