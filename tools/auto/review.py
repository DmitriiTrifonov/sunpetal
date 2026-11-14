#!/usr/bin/env python3
"""Пакет на проверку автора (К-221, А1.1; docs/AUTONOMY.md): `items.json` -> страница -> `answers.json`.

    review.py build PACK_DIR        # PACK_DIR/items.json -> PACK_DIR/index.html (картинки копируются в PACK_DIR/img)
    review.py serve PACK_DIR [--port 8765]   # страница на localhost, ответ щелчком пишется в PACK_DIR/answers.json
    review.py answers PACK_DIR      # напечатать ответы
    review.py apply PACK_DIR        # ответы -> список действий (close / bug / next / pending / decide); сам ничего не меняет

items.json: {"title": "...", "items": [{"id": "k110-1", "card": "K-110", "question": "...", "note": "...",
  "options": [{"id": "yes", "label": "Да"}, ...], "images": [{"path": "a.png", "caption": "DS"}, ...]}]}
Пунктов не больше 10. Необязательно в пункте: "good": [id...] и "bad": [id...] — какие варианты значат «принято»
и «баг» (по умолчанию good: yes/ok/new, bad: no/bug/old; skip — в следующий пакет; прочее — решает агент). Варианты по умолчанию: да / нет / пропустить. Путь картинки — от PACK_DIR или абсолютный.
Страница одна, без внешних ссылок. Без `serve` (file://) ответы копятся в браузере, кнопка «Скачать answers.json».
"""
import html
import http.server
import json
import os
import shutil
import sys

MAX_ITEMS = 10
DEFAULT_OPTIONS = [{'id': 'yes', 'label': 'Да'}, {'id': 'no', 'label': 'Нет'}, {'id': 'skip', 'label': 'Пропустить'}]

CSS = """
:root{--bg:#fafaf7;--fg:#1d1d1b;--mut:#6b6b64;--card:#fff;--bd:#dcdcd2;--ok:#1f7a4d;--no:#b3341f;--sel:#2a5db0}
@media (prefers-color-scheme:dark){:root{--bg:#171715;--fg:#ecece6;--mut:#9a9a90;--card:#222220;--bd:#3a3a36;--ok:#4cc38a;--no:#ef6a54;--sel:#6fa0f0}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.45 system-ui,sans-serif}
main{max-width:980px;margin:0 auto;padding:16px}h1{font-size:1.3rem}
.item{background:var(--card);border:1px solid var(--bd);border-radius:10px;padding:14px;margin:14px 0}
.item h2{font-size:1.05rem;margin:0 0 4px}.card{color:var(--mut);font-size:.85rem}.note{color:var(--mut);font-size:.9rem}
.imgs{display:flex;flex-wrap:wrap;gap:10px;margin:10px 0}.imgs figure{margin:0;flex:1 1 280px}
.imgs img{width:100%;image-rendering:pixelated;border:1px solid var(--bd);border-radius:6px}
figcaption{font-size:.85rem;color:var(--mut);text-align:center}
.opts{display:flex;flex-wrap:wrap;gap:8px}.opts button{font:inherit;padding:8px 16px;border-radius:8px;border:1px solid var(--bd);background:var(--bg);color:var(--fg);cursor:pointer}
.opts button.on{border-color:var(--sel);outline:2px solid var(--sel)}
.bar{position:sticky;bottom:0;background:var(--bg);padding:10px 0;border-top:1px solid var(--bd)}
@media(max-width:600px){main{padding:12px 16px}}
"""

JS = """
const items=%ITEMS%;let ans={};
try{ans=JSON.parse(localStorage.getItem('review:'+document.title)||'{}')}catch(e){}
let live=false;
fetch('answers.json').then(r=>r.ok?r.json():{}).then(a=>{live=true;ans=Object.assign(ans,a);paint()}).catch(()=>paint());
function save(id,v){ans[id]=v;try{localStorage.setItem('review:'+document.title,JSON.stringify(ans))}catch(e){}
 if(live)fetch('answer',{method:'POST',body:JSON.stringify({id:id,answer:v})}).catch(()=>{});paint()}
function paint(){document.querySelectorAll('.opts button').forEach(b=>b.classList.toggle('on',ans[b.dataset.item]===b.dataset.opt));
 const n=Object.keys(ans).length;document.getElementById('count').textContent=n+' из '+items.length}
document.querySelectorAll('.opts button').forEach(b=>b.onclick=()=>save(b.dataset.item,b.dataset.opt));
document.getElementById('dl').onclick=()=>{const a=document.createElement('a');
 a.href=URL.createObjectURL(new Blob([JSON.stringify(ans,null,1)],{type:'application/json'}));a.download='answers.json';a.click()};
paint();
"""


def load(pack):
    with open(os.path.join(pack, 'items.json'), encoding='utf-8') as f:
        doc = json.load(f)
    items = doc['items']
    if not 1 <= len(items) <= MAX_ITEMS:
        raise SystemExit(f'review: пунктов {len(items)}, нужно от 1 до {MAX_ITEMS}')
    ids = [i['id'] for i in items]
    if len(set(ids)) != len(ids):
        raise SystemExit('review: id пунктов повторяются')
    return doc


def build(pack):
    doc = load(pack)
    img_dir = os.path.join(pack, 'img')
    os.makedirs(img_dir, exist_ok=True)
    parts = []
    for n, it in enumerate(doc['items']):
        figs = []
        for k, im in enumerate(it.get('images', [])):
            src = im['path'] if os.path.isabs(im['path']) else os.path.join(pack, im['path'])
            name = f"{it['id']}_{k}{os.path.splitext(src)[1] or '.png'}"
            shutil.copyfile(src, os.path.join(img_dir, name))
            figs.append(f"<figure><img src=\"img/{html.escape(name)}\" alt=\"\"><figcaption>{html.escape(im.get('caption', ''))}</figcaption></figure>")
        opts = ''.join(
            f"<button data-item=\"{html.escape(it['id'])}\" data-opt=\"{html.escape(o['id'])}\">{html.escape(o['label'])}</button>"
            for o in it.get('options', DEFAULT_OPTIONS))
        note = f"<p class=\"note\">{html.escape(it['note'])}</p>" if it.get('note') else ''
        parts.append(
            f"<section class=\"item\"><div class=\"card\">{n + 1}. {html.escape(it.get('card', ''))}</div>"
            f"<h2>{html.escape(it['question'])}</h2>{note}<div class=\"imgs\">{''.join(figs)}</div><div class=\"opts\">{opts}</div></section>")
    title = html.escape(doc.get('title', 'Пакет на проверку'))
    page = (f"<!doctype html><html lang=\"ru\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">"
            f"<title>{title}</title><style>{CSS}</style></head><body><main><h1>{title}</h1>{''.join(parts)}"
            f"<div class=\"bar\">Отвечено: <b id=\"count\"></b> <button id=\"dl\">Скачать answers.json</button></div></main>"
            f"<script>{JS.replace('%ITEMS%', json.dumps([i['id'] for i in doc['items']]))}</script></body></html>")
    with open(os.path.join(pack, 'index.html'), 'w', encoding='utf-8') as f:
        f.write(page)
    return os.path.join(pack, 'index.html')


def read_answers(pack):
    p = os.path.join(pack, 'answers.json')
    if not os.path.exists(p):
        return {}
    with open(p, encoding='utf-8') as f:
        return json.load(f)


GOOD = ('yes', 'ok', 'new')
BAD = ('no', 'bug', 'old')


def apply(pack):
    """Список действий по ответам: close (карточка принята: close-qa), bug (bug-task с тем же кадром),
    next (перенести в следующий пакет), pending (нет ответа), decide (вариант без толкования)."""
    ans = read_answers(pack)
    out = []
    for it in load(pack)['items']:
        a = ans.get(it['id'])
        good, bad = it.get('good', GOOD), it.get('bad', BAD)
        kind = ('pending' if a is None else 'close' if a in good else 'bug' if a in bad
                else 'next' if a == 'skip' else 'decide')
        out.append({'id': it['id'], 'card': it.get('card', ''), 'answer': a, 'action': kind,
                    'question': it['question'], 'images': [im['path'] for im in it.get('images', [])]})
    return out


def write_answer(pack, item, answer):
    ans = read_answers(pack)
    ans[item] = answer
    tmp = os.path.join(pack, 'answers.json.tmp')
    with open(tmp, 'w', encoding='utf-8') as f:
        json.dump(ans, f, ensure_ascii=False, indent=1)
    os.replace(tmp, os.path.join(pack, 'answers.json'))


def handler(pack):
    ids = {i['id'] for i in load(pack)['items']}

    class H(http.server.SimpleHTTPRequestHandler):
        def __init__(self, *a, **k):
            super().__init__(*a, directory=pack, **k)

        def do_POST(self):
            try:
                body = json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0))))
                if self.path != '/answer' or body['id'] not in ids:
                    raise ValueError
                write_answer(pack, body['id'], body['answer'])
                self.send_response(204)
            except (ValueError, KeyError):
                self.send_response(400)
            self.end_headers()

        def log_message(self, *a):
            pass
    return H


def main(argv):
    if len(argv) < 2 or argv[0] not in ('build', 'serve', 'answers', 'apply'):
        print(__doc__)
        return 2
    cmd, pack = argv[0], argv[1]
    if cmd == 'build':
        print(build(pack))
    elif cmd == 'apply':
        for a in apply(pack):
            print(f"{a['action']:8} {a['id']:12} {a['card']:20} {a['answer']}")
    elif cmd == 'answers':
        print(json.dumps(read_answers(pack), ensure_ascii=False, indent=1))
    else:
        port = int(argv[argv.index('--port') + 1]) if '--port' in argv else 8765
        build(pack)
        print(f'http://localhost:{port}/index.html  (ответы: {pack}/answers.json)')
        http.server.ThreadingHTTPServer(('127.0.0.1', port), handler(pack)).serve_forever()
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
