import http.client
import json
import os
import re
import sys
import tempfile
import threading
import unittest
import http.server

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import review

PNG = bytes.fromhex('89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000d49444154789c6360000002000001e221bc330000000049454e44ae426082')


def pack(n=3):
    d = tempfile.mkdtemp()
    open(os.path.join(d, 'a.png'), 'wb').write(PNG)
    items = [{'id': f'i{k}', 'card': 'K-1', 'question': f'Вопрос {k}?', 'images': [{'path': 'a.png', 'caption': 'DS'}]} for k in range(n)]
    json.dump({'title': 'Тест', 'items': items}, open(os.path.join(d, 'items.json'), 'w'))
    return d


class T(unittest.TestCase):
    def test_build_has_no_external_links(self):
        d = pack()
        page = open(review.build(d), encoding='utf-8').read()
        self.assertEqual(page.count('<section'), 3)
        self.assertFalse(re.findall(r'(?:src|href)="https?:', page))
        self.assertTrue(os.path.exists(os.path.join(d, 'img', 'i0_0.png')))

    def test_limit(self):
        with self.assertRaises(SystemExit):
            review.build(pack(11))

    def test_answer_roundtrip(self):
        d = pack()
        review.build(d)
        srv = http.server.ThreadingHTTPServer(('127.0.0.1', 0), review.handler(d))
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        c = http.client.HTTPConnection('127.0.0.1', srv.server_port)
        c.request('POST', '/answer', json.dumps({'id': 'i1', 'answer': 'no'}))
        self.assertEqual(c.getresponse().status, 204)
        c.request('POST', '/answer', json.dumps({'id': 'zzz', 'answer': 'no'}))
        self.assertEqual(c.getresponse().status, 400)
        srv.shutdown()
        self.assertEqual(review.read_answers(d), {'i1': 'no'})


    def test_apply_maps_three_answers(self):
        d = pack(4)
        doc = json.load(open(os.path.join(d, 'items.json')))
        doc['items'][3]['options'] = [{'id': 'a', 'label': 'А'}]
        json.dump(doc, open(os.path.join(d, 'items.json'), 'w'))
        for k, v in (('i0', 'yes'), ('i1', 'no'), ('i2', 'skip')):
            review.write_answer(d, k, v)
        self.assertEqual([a['action'] for a in review.apply(d)], ['close', 'bug', 'next', 'pending'])
        review.write_answer(d, 'i3', 'a')
        self.assertEqual(review.apply(d)[3]['action'], 'decide')


if __name__ == '__main__':
    unittest.main()
