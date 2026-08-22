"""Rebuild the renderer payload from the wiki art set.

The art host used until now (coc.guide) predates TH17/18: five structures had
no portrait at any level, and portraits existed only at levels where the
appearance changes, so most buildings were drawn at a lower tier than they are.

The game wiki carries one file per level for every structure — Cannon 1-21,
Wall 1-19, Town Hall 1-18 — including the buildings that were missing outright.
It also publishes walls as a *single block per level* rather than the two-tile
corner icon the shop art uses, which is what makes real wall art tileable.
"""
import json, io, os, base64, collections
from PIL import Image

jobs = json.load(open('fetch_jobs.json'))          # (name, level, wiki file)
old  = json.load(open('payload19.json'))
war  = json.load(open('layouts_war_v7.json'))
farm = json.load(open('layouts_farm_v7.json'))

BOX = 96          # a little larger than the old 80: the wiki art is sharper

def key_for(f): return 'w_' + f[:-4].replace(' ', '_')

imgs, asp = {}, {}
for f in sorted({j[2] for j in jobs}):
    p = os.path.join('wikiart', f)
    im = Image.open(p).convert('RGBA')
    bb = im.getbbox()
    if bb: im = im.crop(bb)
    s = BOX / max(im.size)
    if s < 1:
        im = im.resize((max(1, round(im.width*s)), max(1, round(im.height*s))), Image.LANCZOS)
    buf = io.BytesIO(); im.save(buf, 'PNG', optimize=True)
    k = key_for(f)
    imgs[k] = 'data:image/png;base64,' + base64.b64encode(buf.getvalue()).decode()
    asp[k] = list(im.size)

art_of = {(n, lv): key_for(f) for n, lv, f in jobs}
exact_of = {}
wmap = json.load(open('wiki_map.json'))
for n, lv, f in jobs:
    e = wmap[n]
    # Exact when the wiki actually publishes this level (or the art carries no
    # level at all, in which case one file is the whole truth for it).
    exact_of[(n, lv)] = bool(e.get('single')) or str(lv) in e['levels']

def pack(l):
    b = []
    for p in l['placements']:
        r = p['rect']
        k = art_of.get((p['name'], p['level']))
        ex = exact_of.get((p['name'], p['level']), False)
        b.append([p['name'], p['level'], r['x'], r['y'], r['w'], r['h'], p['class'],
                  p['range'], p['min_range'], 1 if p['is_trap'] else 0, k, 1 if ex else 0])
    wl = l['wall_level']
    return {'b': b, 'w': [list(t) for t in l['walls']], 'wl': wl,
            'wimg': art_of.get(('Wall', wl))}

def build(doc):
    res = {}; rnd = lambda m: {k: round(v, 4) for k, v in m.items()}
    for th, e in doc['townhalls'].items():
        o = pack(e['layout']); sd = pack(e['seed_layout'])
        res[th] = {'th': int(th), 'score': round(e['score'], 4),
                   'seed_score': round(e['seed_score'], 4),
                   'gain': round(e['gain_percent'], 1),
                   'm': rnd(e['metrics']), 'sm': rnd(e['seed_metrics']),
                   'opt': o, 'seed': sd,
                   'art': {'exact': sum(1 for r in o['b'] if r[11]),
                           'near':  sum(1 for r in o['b'] if r[10] and not r[11]),
                           'none':  sum(1 for r in o['b'] if not r[10])}}
    return res

pay = {'war': build(war), 'farm': build(farm), 'imgs': imgs, 'asp': asp,
       'lib': old['lib'], 'version': old['version'],
       'fingerprint': old['fingerprint'], 'wallpal': old['wallpal']}
json.dump(pay, open('payload20.json', 'w'), separators=(',', ':'))

print('art keys:', len(imgs))
for th in ('1', '9', '15', '17', '18'):
    e = pay['war'][th]
    print(f"TH{th:>2} art {e['art']}  wall sprite {e['opt']['wimg']}")
print('MB', round(os.path.getsize('payload20.json')/1e6, 2))
