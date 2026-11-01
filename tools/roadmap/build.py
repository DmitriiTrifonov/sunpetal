#!/usr/bin/env python3
"""Собирает docs/ROADMAP.html из tools/roadmap/{roadmap.json,done.json,timelog.csv,history.csv}.

Две вкладки: «Осталось» (открытое, с весами) и «Всё дерево» (сделано + в работе + открыто, с фильтрами).
Только стандартная библиотека. Запуск: python3 tools/roadmap/build.py [--out F]
"""
import argparse
import csv
import html
import json
import sys
from datetime import date
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
VALID_W = {1, 2, 3, 5, 8, 13}
TAG_LABEL = {"manual": "ручная", "author": "автор", "deferred": "отложено", "close": "закрыть"}
ST_LABEL = {"done": "сделано", "wip": "в работе", "partial": "частично", "open": "открыто"}
e = html.escape


def load_times():
    rows = []
    for name in ("timelog.csv", "history.csv"):
        path = HERE / name
        if path.exists():
            with path.open(encoding="utf-8", newline="") as f:
                rows += list(csv.DictReader(f))
    return rows


def leaves(node):
    if node.get("c"):
        for k in node["c"]:
            yield from leaves(k)
    else:
        yield node


def prepare(node, path, errors, status, wip_ids):
    """Проставляет статус листьям и считает по веткам: вес и число шагов по статусам."""
    node["_path"] = path
    kids = node.get("c")
    if kids:
        node["_by"] = {s: [0, 0] for s in ST_LABEL}
        for i, kid in enumerate(kids):
            prepare(kid, f"{path}.{i + 1}", errors, status, wip_ids)
            for s, (n, w) in kid["_by"].items():
                node["_by"][s][0] += n
                node["_by"][s][1] += w
        node["_n"] = sum(v[0] for v in node["_by"].values())
        node["_w"] = sum(v[1] for v in node["_by"].values())
        return
    w = node.get("w")
    if w not in VALID_W:
        errors.append(f"{path} «{node.get('t')}»: вес {w!r} не из {sorted(VALID_W)}")
        w = 0
    for tag in node.get("k", []):
        if tag not in TAG_LABEL:
            errors.append(f"{path}: неизвестная метка {tag!r}")
    st = status
    if status == "open":
        if node.get("id") in wip_ids:
            st = "wip"
        elif node.get("p"):
            st = "partial"
    node["_st"] = st
    node["_w"] = w
    node["_n"] = 1
    node["_by"] = {s: [0, 0] for s in ST_LABEL}
    node["_by"][st] = [1, w]


def merge(open_tree, done_tree):
    """Сливает ветки с одинаковым названием: сначала сделанное, потом открытое."""
    done = {b["t"]: b for b in done_tree}
    out = []
    for b in open_tree:
        d = done.pop(b["t"], None)
        out.append({**b, "c": (d["c"] if d else []) + b["c"]} if d else b)
    out += done.values()
    return out


def render(node, mode, measured, unique):
    kids = node.get("c")
    if kids:
        by = node["_by"]
        inner = "".join(render(k, mode, measured, unique) for k in kids)
        src = f'<span class="src">{e(node["s"])}</span>' if node.get("s") else ""
        depth = node["_path"].count(".")
        if mode == "all":
            dn, dw = by["done"]
            frac = 100 * dw / node["_w"] if node["_w"] else 0
            extra = f'<span class="cnt">сделано {dn} из {node["_n"]} · {frac:.0f}% веса</span>'
            bar = f'<span class="bar"><i style="width:{frac:.1f}%"></i></span>'
        else:
            extra = f'<span class="cnt">{node["_n"]} шаг.</span>'
            bar = ""
        return (
            f'<details class="branch d{min(depth, 2)}"{" open" if depth == 0 else ""}><summary>'
            f'<span class="w sum">{node["_w"]}</span><span class="title">{e(node["t"])}</span>'
            f'{extra}{src}{bar}</summary>{inner}</details>'
        )
    tags = node.get("k", [])
    st = node["_st"]
    chips = ""
    if mode == "all":
        chips += f'<span class="st {st}">{ST_LABEL[st]}</span>'
    chips += "".join(f'<span class="tag {t}">{TAG_LABEL[t]}</span>' for t in tags)
    if mode == "open" and node.get("p"):
        chips += '<span class="tag partial">частично</span>'
    hours = measured.get(node.get("id"))
    if hours and unique.get(node["id"]) == 1:
        chips += f'<span class="tag meas">{hours[0]:.1f} ч{" ≈" if hours[1] else " факт"}</span>'
    note = f'<span class="note">{e(node["n"])}</span>' if node.get("n") else ""
    src = f'<span class="src">{e(node["s"])}</span>' if node.get("s") else ""
    return (
        f'<div class="leaf" data-tags="{" ".join(tags)}" data-st="{st}">'
        f'<span class="w w{node["_w"]}">{node["_w"]}</span>'
        f'<span class="title">{e(node["t"])}</span>{chips}{src}{note}</div>'
    )


def graph_data(node, name=None):
    if node.get("c"):
        return {"t": name or node["t"], "w": node["_w"], "n": node["_n"],
                "by": {k: v for k, v in node["_by"].items()},
                "c": [graph_data(k) for k in node["c"]]}
    return {"t": node["t"], "w": node["_w"], "st": node["_st"], "stl": ST_LABEL[node["_st"]], "k": node.get("k", []), "note": node.get("n", "")}


def times_html(rows):
    body = "".join(
        f"<tr><td>{e(r['id'])}</td><td>{e(r['title'])}</td><td>{e(r['start'][:16].replace('T', ' '))}</td>"
        f"<td>{e((r['end'] or 'идёт')[:16].replace('T', ' '))}</td><td class=\"num\">{e(r['hours'] or '…')}</td>"
        f"<td>{'живой' if r['source'] == 'live' else 'git'}</td></tr>"
        for r in sorted(rows, key=lambda r: r["start"])
    )
    return ('<details class="branch d0"><summary><span class="title">Замеры времени</span>'
            f'<span class="cnt">{len(rows)} записей</span></summary>'
            '<table><tr><th>id</th><th>этап</th><th>начало</th><th>конец</th><th>часы</th><th>источник</th></tr>'
            f"{body}</table></details>")


PAGE = """<!DOCTYPE html>
<html lang="ru"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Sunpetal: дерево работ</title>
<style>
:root{--bg:#fbfaf7;--fg:#22201c;--mut:#7a746a;--line:#e3ddd0;--card:#fff;--acc:#d9822b;
--w1:#cfe8d5;--w2:#dfeec2;--w3:#f4e7a8;--w5:#f6cf9a;--w8:#f2a98e;--w13:#e68a9c}
@media(prefers-color-scheme:dark){:root{--bg:#1b1a17;--fg:#ece8df;--mut:#9a9488;--line:#3a372f;--card:#24221e;--acc:#e8a05a;
--w1:#2f4a38;--w2:#44512a;--w3:#5a5127;--w5:#6b4a25;--w8:#7a3d2b;--w13:#7a2c40}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.45 system-ui,-apple-system,"Segoe UI",sans-serif}
main{max-width:1000px;margin:0 auto;padding:28px 18px 80px}
h1{margin:0 0 4px;font-size:26px}.sub{color:var(--mut);margin:0 0 18px}
.stats{display:flex;flex-wrap:wrap;gap:10px;margin-bottom:16px}
.stat{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:8px 14px}
.stat b{display:block;font-size:22px}.stat span{color:var(--mut);font-size:12px}
.tabs{display:flex;gap:4px;border-bottom:2px solid var(--line);margin-bottom:12px}
.tabs button{font:inherit;font-weight:600;border:0;background:none;color:var(--mut);padding:8px 16px;cursor:pointer;border-bottom:3px solid transparent;margin-bottom:-2px}
.tabs button.on{color:var(--fg);border-color:var(--acc)}
.tab{display:none}.tab.on{display:block}
.tools{display:flex;flex-wrap:wrap;gap:14px;align-items:center;margin:10px 0 14px;padding:10px 14px;background:var(--card);border:1px solid var(--line);border-radius:10px}
.tools label{cursor:pointer;user-select:none}.tools button,.tools select,.tools input[type=search]{font:inherit;border:1px solid var(--line);background:none;color:var(--fg);border-radius:6px;padding:3px 10px}
.tools button{cursor:pointer}.tools .sep{width:1px;height:20px;background:var(--line)}
.legend{display:flex;gap:6px;align-items:center;color:var(--mut);font-size:12px}
details{margin:4px 0}details details,.leaf{margin-left:18px}
summary{list-style:none;cursor:pointer;display:flex;flex-wrap:wrap;align-items:center;gap:8px;padding:7px 10px;background:var(--card);border:1px solid var(--line);border-radius:8px}
summary::-webkit-details-marker{display:none}
summary::before{content:"▸";color:var(--mut);width:10px}details[open]>summary::before{content:"▾"}
.d0>summary{font-size:16.5px;font-weight:600;border-left:4px solid var(--acc)}.d1>summary{font-weight:600}
.w{flex:none;min-width:26px;text-align:center;border-radius:6px;padding:1px 6px;font-weight:700;font-size:13px;color:#22201c}
.sum{background:var(--acc);color:#fff;min-width:38px}
.w1{background:var(--w1)}.w2{background:var(--w2)}.w3{background:var(--w3)}.w5{background:var(--w5)}.w8{background:var(--w8)}.w13{background:var(--w13)}
@media(prefers-color-scheme:dark){.w{color:#ece8df}}
.title{flex:1 1 280px}.cnt,.src{color:var(--mut);font-size:12px}
.bar{flex:0 0 90px;height:6px;background:var(--line);border-radius:3px;overflow:hidden}.bar i{display:block;height:100%;background:var(--acc)}
.leaf{display:flex;flex-wrap:wrap;align-items:center;gap:8px;padding:6px 10px;border-left:2px solid var(--line);margin-top:3px}
.leaf[data-st=done] .title{color:var(--mut)}
.note{flex:1 0 100%;padding-left:34px;color:var(--mut);font-size:13px}
.tag,.st{font-size:11px;padding:1px 7px;border-radius:10px;border:1px solid var(--line);color:var(--mut)}
.st.done{background:#4aa86a;border-color:#4aa86a;color:#fff}.st.wip{background:#5b8def;border-color:#5b8def;color:#fff}
.st.partial{border-color:var(--acc);color:var(--acc)}.st.open{}
.tag.manual{border-color:#5b8def;color:#5b8def}.tag.author{border-color:#c0559a;color:#c0559a}
.tag.deferred{border-color:#8a8a8a;text-decoration:line-through}.tag.close{border-color:#4aa86a;color:#4aa86a}.tag.partial{border-color:var(--acc);color:var(--acc)}
.tag.meas{border-color:#4aa86a;color:#4aa86a;font-weight:600}
table{border-collapse:collapse;width:100%;margin:8px 0;font-size:13px}th,td{border-bottom:1px solid var(--line);padding:4px 8px;text-align:left}.num{text-align:right}
.hidden{display:none!important}
footer{margin-top:30px;color:var(--mut);font-size:12px}
#graph{height:72vh;min-height:420px;border:1px solid var(--line);border-radius:10px;background:var(--card);overflow:hidden;cursor:grab}
#graph:active{cursor:grabbing}#gsvg{width:100%;height:100%;display:block;user-select:none}
.ge{fill:none;stroke:var(--line);stroke-width:1.5}.gl{font-size:12px;fill:var(--fg)}.gl.m{fill:var(--mut)}
.gn{stroke:var(--card);stroke-width:1.5}.gn.done{fill:#4aa86a}.gn.wip{fill:#5b8def}.gn.partial{fill:var(--acc)}.gn.open{fill:var(--mut)}
.gb{fill:var(--card);stroke:var(--line);stroke-width:2;cursor:pointer}.ga{fill:none;stroke:#4aa86a;stroke-width:4}
.gr{fill:var(--acc)}.gt{font-size:12px;font-weight:600;fill:var(--fg);cursor:pointer}
</style></head><body><main>
<h1>Sunpetal: дерево работ</h1>
<p class="sub">Собрано __DATE__ из <code>docs/PLAN.md</code>, <code>SINGLE-SCREEN.md</code>, <code>2D-UPSCALE.md</code>, <code>BUGS.md</code>, <code>STATUS.md</code> и истории git; снимок плана от __UPDATED__. Число в цветном квадрате — вес сложности, в оранжевом — сумма ветки.</p>
<div class="stats">__STATS__</div>
<div class="tabs"><button class="on" data-tab="open">Осталось</button><button data-tab="all">Всё дерево</button><button data-tab="graph">Граф</button></div>

<section class="tab on" id="tab-open" data-tree>
<div class="tools">
<span class="legend">Вес: <span class="w w1">1</span><span class="w w2">2</span><span class="w w3">3</span><span class="w w5">5</span><span class="w w8">8</span><span class="w w13">13</span></span>
<label><input type="checkbox" data-hide="manual"> скрыть «ручная»</label>
<label><input type="checkbox" data-hide="author"> скрыть «автор»</label>
<label><input type="checkbox" data-hide="deferred" checked> скрыть «отложено»</label>
<label><input type="checkbox" data-hide="close"> скрыть «закрыть»</label>
<button data-open>развернуть всё</button><button data-shut>свернуть</button>
</div>__TREE_OPEN__</section>

<section class="tab" id="tab-all" data-tree>
<div class="tools">
<span class="legend">Показать:</span>
<label><input type="checkbox" data-st="done" checked> сделано</label>
<label><input type="checkbox" data-st="wip" checked> в работе</label>
<label><input type="checkbox" data-st="partial" checked> частично</label>
<label><input type="checkbox" data-st="open" checked> открыто</label>
<span class="sep"></span>
<label><input type="checkbox" data-hide="manual"> скрыть «ручная»</label>
<label><input type="checkbox" data-hide="author"> скрыть «автор»</label>
<label><input type="checkbox" data-hide="deferred"> скрыть «отложено»</label>
<span class="sep"></span>
<input type="search" placeholder="поиск…" data-q>
<button data-open>развернуть всё</button><button data-shut>свернуть</button>
</div>__TREE_ALL__</section>

<section class="tab" id="tab-graph">
<div class="tools">
<span class="legend">Узлы:</span>
<label><input type="checkbox" data-gs="done" checked> сделано</label>
<label><input type="checkbox" data-gs="wip" checked> в работе</label>
<label><input type="checkbox" data-gs="partial" checked> частично</label>
<label><input type="checkbox" data-gs="open" checked> открыто</label>
<span class="sep"></span>
<label><input type="checkbox" data-gh="manual"> скрыть «ручная»</label>
<label><input type="checkbox" data-gh="author"> скрыть «автор»</label>
<label><input type="checkbox" data-gh="deferred"> скрыть «отложено»</label>
<label><input type="checkbox" data-gh="close"> скрыть «закрыть»</label>
<label>вес ≥ <select id="g-wmin"><option>1</option><option>2</option><option>3</option><option>5</option><option>8</option><option>13</option></select></label>
<input type="search" id="g-q" placeholder="поиск…">
<span class="sep"></span>
<span class="legend">масштаб:</span><button id="g-out">−</button><span id="g-zoom" class="legend" style="min-width:42px;text-align:center">100%</span><button id="g-in">+</button>
<button id="g-fit">вписать</button><button id="g-100">100%</button>
<button id="g-exp">развернуть всё</button><button id="g-col">свернуть ветки</button>
<span class="legend">клик по ветке — свернуть/развернуть; колесо — масштаб; перетаскивание — сдвиг. Размер узла — вес, кольцо ветки — доля сделанного.</span>
</div>
<div id="graph"><svg id="gsvg"><g id="gview"></g></svg></div>
</section>

__TIMES__
<footer>Шкала: __SCALE__<br>Веса сделанного поставлены задним числом (оценка); часы «≈» — из истории git (нижняя оценка), «факт» — из <code>timelog.csv</code>. Файл создан <code>tools/roadmap/build.py</code>; пересоздание — скилл <code>roadmap-tree</code>. Руками не править.</footer>
</main>
<script>
function wire(tab){
  const hide=new Set(), st=new Set(['done','wip','partial','open']);let q='';
  tab.querySelectorAll('[data-hide]').forEach(c=>{if(c.checked)hide.add(c.dataset.hide);
    c.onchange=()=>{c.checked?hide.add(c.dataset.hide):hide.delete(c.dataset.hide);apply()}});
  tab.querySelectorAll('input[data-st]').forEach(c=>{
    c.onchange=()=>{c.checked?st.add(c.dataset.st):st.delete(c.dataset.st);apply()}});
  const s=tab.querySelector('[data-q]');if(s)s.oninput=()=>{q=s.value.trim().toLowerCase();apply()};
  tab.querySelector('[data-open]').onclick=()=>tab.querySelectorAll('details').forEach(d=>d.open=true);
  tab.querySelector('[data-shut]').onclick=()=>tab.querySelectorAll('details').forEach(d=>d.open=false);
  function apply(){
    tab.querySelectorAll('.leaf').forEach(l=>{
      const t=(l.dataset.tags||'').split(' ').filter(Boolean);
      const bad=t.some(x=>hide.has(x))||!st.has(l.dataset.st)||(q&&!l.textContent.toLowerCase().includes(q));
      l.classList.toggle('hidden',bad)});
    tab.querySelectorAll('details.branch').forEach(d=>d.classList.toggle('hidden',!d.querySelector('.leaf:not(.hidden)')));
    if(q)tab.querySelectorAll('details.branch:not(.hidden)').forEach(d=>d.open=true);
  }
  apply();
}
document.querySelectorAll('.tab[data-tree]').forEach(wire);
document.querySelectorAll('.tabs button').forEach(b=>b.onclick=()=>{
  document.querySelectorAll('.tabs button').forEach(x=>x.classList.toggle('on',x===b));
  document.querySelectorAll('.tab').forEach(t=>t.classList.toggle('on',t.id==='tab-'+b.dataset.tab));
  if(b.dataset.tab==='graph')window.graphShown();
});

const G=__GRAPH__;
(function(){
  const svg=document.getElementById('gsvg'),view=document.getElementById('gview'),NS='http://www.w3.org/2000/svg';
  const HID=new Set();let WMIN=1,Q='';
  const ST=new Set(['done','wip','partial','open']),COLX=[0,340,660],ROW=24;
  let tx=20,ty=20,k=1,fitted=false;
  function mk(tag,attrs,text){const n=document.createElementNS(NS,tag);for(const a in attrs)n.setAttribute(a,attrs[a]);if(text!=null)n.textContent=text;return n}
  function vis(n){if(n.c)return n.c.some(vis);return ST.has(n.st)&&n.w>=WMIN&&!n.k.some(x=>HID.has(x))&&(!Q||n.t.toLowerCase().includes(Q))}
  function cut(t,m){return t.length>m?t.slice(0,m-1)+'…':t}
  function layout(n,d,st){
    n.d=d;n.x=COLX[Math.min(d,2)];
    if(!n.c||(n.col&&!Q)){n.y=st.row++*ROW;n.shown=true;return}
    const kids=n.c.filter(vis);kids.forEach(c=>layout(c,d+1,st));
    n.kids=kids;n.y=kids.length?(kids[0].y+kids[kids.length-1].y)/2:st.row++*ROW;
  }
  function draw(){
    view.textContent='';const st={row:0};layout(G,0,st);
    const eg=mk('g',{}),ng=mk('g',{});view.append(eg,ng);
    (function walk(n){
      const kids=(n.c&&!(n.col&&!Q))?n.kids:[];
      kids.forEach(c=>{const mx=(n.x+c.x)/2;
        eg.append(mk('path',{class:'ge',d:`M${n.x},${n.y}C${mx},${n.y} ${mx},${c.y} ${c.x},${c.y}`}));walk(c)});
      if(n.c){
        const r=Math.min(22,7+Math.sqrt(n.w)*0.9),circ=2*Math.PI*r,f=n.t==='Sunpetal'?0:(n.by.done[1]/(n.w||1));
        const g=mk('g',{});
        g.append(mk('circle',{class:'gb',cx:n.x,cy:n.y,r}));
        if(f>0)g.append(mk('circle',{class:'ga',cx:n.x,cy:n.y,r,'stroke-dasharray':`${circ*f} ${circ}`,transform:`rotate(-90 ${n.x} ${n.y})`}));
        if(n.d>0){
          g.append(mk('text',{class:'gt',x:n.x-r-6,y:n.y+4,'text-anchor':'end'},cut(n.t,34)+(n.col&&!Q?` (+${n.n})`:'')));
          g.append(mk('title',{},`${n.t}\nсделано ${n.by.done[0]} из ${n.n}, вес ${n.w}`));
          g.onclick=ev=>{n.col=!n.col;draw()};
        } else {g.append(mk('text',{class:'gt',x:n.x,y:n.y+4,'text-anchor':'middle'},'Sunpetal'))}
        ng.append(g);
      } else {
        const r=3+Math.sqrt(n.w)*1.6;
        const c=mk('circle',{class:'gn '+n.st,cx:n.x,cy:n.y,r});
        c.append(mk('title',{},`${n.t}\nвес ${n.w} · ${n.stl}${n.note?' · '+n.note:''}`));
        ng.append(c);
        ng.append(mk('text',{class:'gl'+(n.st==='done'?' m':''),x:n.x+r+6,y:n.y+4},cut(n.t,70)));
      }
    })(G);
    apply();
  }
  function apply(){view.setAttribute('transform',`translate(${tx},${ty}) scale(${k})`);document.getElementById('g-zoom').textContent=Math.round(k*100)+'%'}
  const LEVELS=[0.25,0.4,0.55,0.75,1,1.4,2,3];
  function step(dir,px,py){const nk=dir>0?LEVELS.find(l=>l>k*1.02):[...LEVELS].reverse().find(l=>l<k/1.02);if(nk)zoomTo(nk,px,py)}
  function zoomTo(nk,px,py){tx=px-(px-tx)*nk/k;ty=py-(py-ty)*nk/k;k=nk;apply()}
  function fit(){
    const b=view.getBBox(),w=svg.clientWidth,h=svg.clientHeight;if(!w||!b.width)return;
    k=Math.max(0.55,Math.min(w/(b.width+60),h/(b.height+40),1.2));tx=-b.x*k+30;ty=-b.y*k+20;apply();
  }
  window.graphShown=()=>{if(!fitted){fitted=true;fit()}};
  svg.addEventListener('wheel',ev=>{ev.preventDefault();const r=svg.getBoundingClientRect();const t=Date.now();if(t-lastWheel<180)return;lastWheel=t;step(ev.deltaY<0?1:-1,ev.clientX-r.left,ev.clientY-r.top)},{passive:false});
  let lastWheel=0;
  const mid=()=>[svg.clientWidth/2,svg.clientHeight/2];
  document.getElementById('g-in').onclick=()=>step(1,...mid());
  document.getElementById('g-out').onclick=()=>step(-1,...mid());
  document.getElementById('g-100').onclick=()=>zoomTo(1,...mid());
  document.querySelectorAll('[data-gh]').forEach(c=>c.onchange=()=>{c.checked?HID.add(c.dataset.gh):HID.delete(c.dataset.gh);draw()});
  document.getElementById('g-wmin').onchange=ev=>{WMIN=+ev.target.value;draw()};
  document.getElementById('g-q').oninput=ev=>{Q=ev.target.value.trim().toLowerCase();draw();fit()};
  let drag=null;
  svg.addEventListener('pointerdown',ev=>{drag={x:ev.clientX-tx,y:ev.clientY-ty}});
  addEventListener('pointermove',ev=>{if(drag){tx=ev.clientX-drag.x;ty=ev.clientY-drag.y;apply()}});
  addEventListener('pointerup',()=>drag=null);
  document.querySelectorAll('[data-gs]').forEach(c=>c.onchange=()=>{c.checked?ST.add(c.dataset.gs):ST.delete(c.dataset.gs);draw()});
  document.getElementById('g-fit').onclick=fit;
  document.getElementById('g-exp').onclick=()=>{(function z(n){n.col=false;(n.c||[]).forEach(z)})(G);draw();fit()};
  document.getElementById('g-col').onclick=()=>{G.c.forEach(b=>b.col=true);draw();fit()};
  G.c.forEach(b=>b.col=true);
  draw();
})();
</script></body></html>
"""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(ROOT / "docs/ROADMAP.html"))
    args = ap.parse_args()

    data = json.loads((HERE / "roadmap.json").read_text(encoding="utf-8"))
    done_path = HERE / "done.json"
    done = json.loads(done_path.read_text(encoding="utf-8"))["tree"] if done_path.exists() else []

    times = load_times()
    wip_ids = {r["id"] for r in times if r["source"] == "live" and not r["end"]}
    measured = {}  # id -> [часы, из git?]
    for r in times:
        if r["hours"]:
            m = measured.setdefault(r["id"], [0.0, False])
            m[0] += float(r["hours"])
            m[1] = m[1] or r["source"] == "git"

    errors = []
    open_root = {"t": "root", "c": data["tree"]}
    prepare(open_root, "0", errors, "open", wip_ids)
    all_tree = merge(json.loads(json.dumps(data["tree"])), done)
    all_root = {"t": "root", "c": all_tree}
    # статус done у листьев из done.json: у открытых веток он «open», поэтому считаем раздельно
    prepare_all(all_root, errors, set(map(id, ())), done, data["tree"], wip_ids)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1

    unique = {}
    for n in leaves(all_root):
        if n.get("id"):
            unique[n["id"]] = unique.get(n["id"], 0) + 1
    wsum = {}
    for n in leaves(all_root):
        if n.get("id") in measured:
            wsum[n["id"]] = wsum.get(n["id"], 0) + n["_w"]
    cal_h = sum(measured[i][0] for i in wsum)
    cal_w = sum(wsum.values())

    by = open_root["_by"]
    total = open_root["_w"]
    manual = sum(n["_w"] for n in leaves(open_root) if "manual" in n.get("k", []))
    author = sum(n["_w"] for n in leaves(open_root) if "author" in n.get("k", []))
    auto = sum(n["_w"] for n in leaves(open_root) if not {"manual", "author", "deferred", "close"} & set(n.get("k", [])))
    dn = all_root["_by"]["done"]
    stats = [
        (total, "вес осталось"), (open_root["_n"], "шагов осталось"), (auto, "из них автономно"),
        (manual, "требуют ручной проверки"), (author, "ждут автора"),
        (f"{dn[1]}", f"вес сделано ({dn[0]} шагов, оценка задним числом)"),
        (f"{sum(m[0] for m in measured.values()):.0f}", "ч замерено (git ≈ + живые)"),
        (f"{cal_h / cal_w:.2f}" if cal_w else "—", f"ч на единицу веса ({len(wsum)} этапов)" if cal_w else "ч на единицу веса"),
    ]
    page = (
        PAGE.replace("__DATE__", date.today().isoformat())
        .replace("__UPDATED__", e(data.get("updated", "?")))
        .replace("__STATS__", "".join(f'<div class="stat"><b>{v}</b><span>{k}</span></div>' for v, k in stats))
        .replace("__TREE_OPEN__", "".join(render(b, "open", measured, unique) for b in open_root["c"]))
        .replace("__TREE_ALL__", "".join(render(b, "all", measured, unique) for b in all_root["c"]))
        .replace("__TIMES__", times_html(times))
        .replace("__GRAPH__", json.dumps(graph_data(all_root, "Sunpetal"), ensure_ascii=False))
        .replace("__SCALE__", e(data.get("scale", "")))
    )
    Path(args.out).write_text(page, encoding="utf-8")
    print(f"{args.out}: осталось вес {total} ({open_root['_n']} шаг.), сделано вес {dn[1]} ({dn[0]} шаг.), "
          f"калибровка {'%.2f ч/ед' % (cal_h / cal_w) if cal_w else 'нет данных'}")
    return 0


def prepare_all(root, errors, _unused, done_tree, open_tree, wip_ids):
    """Слитое дерево: листья из done.json — «done», остальные — по обычным правилам открытых."""
    done_leaves = {id(n) for b in done_tree for n in leaves(b)}
    def go(node, path):
        node["_path"] = path
        kids = node.get("c")
        if kids:
            node["_by"] = {s: [0, 0] for s in ST_LABEL}
            for i, k in enumerate(kids):
                go(k, f"{path}.{i + 1}")
                for s, (n, w) in k["_by"].items():
                    node["_by"][s][0] += n
                    node["_by"][s][1] += w
            node["_n"] = sum(v[0] for v in node["_by"].values())
            node["_w"] = sum(v[1] for v in node["_by"].values())
            return
        w = node.get("w")
        if w not in VALID_W:
            errors.append(f"{path} «{node.get('t')}»: вес {w!r} не из {sorted(VALID_W)}")
            w = 0
        if id(node) in done_leaves:
            st = "done"
        elif node.get("id") in wip_ids:
            st = "wip"
        elif node.get("p"):
            st = "partial"
        else:
            st = "open"
        node["_st"], node["_w"], node["_n"] = st, w, 1
        node["_by"] = {s: [0, 0] for s in ST_LABEL}
        node["_by"][st] = [1, w]
    go(root, "0")


if __name__ == "__main__":
    sys.exit(main())
