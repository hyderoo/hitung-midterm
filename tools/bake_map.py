#!/usr/bin/env python3
"""Bake data/map.json (SVG path + label anchor per state) from the US Atlas
TopoJSON (us-atlas, public domain, derived from Census cartographic boundaries).
Run: python3 tools/bake_map.py"""
import json, os
here = os.path.dirname(__file__)
T = json.load(open(os.path.join(here, "states.topo.json")))
sc, tr = T["transform"]["scale"], T["transform"]["translate"]
arcs = []
for a in T["arcs"]:
    x = y = 0
    p = []
    for dx, dy in a:
        x += dx; y += dy
        p.append((x * sc[0] + tr[0], y * sc[1] + tr[1]))
    arcs.append(p)
def ring(r):
    p = []
    for i in r:
        q = arcs[i] if i >= 0 else arcs[~i][::-1]
        p += q if not p else q[1:]
    return p
AB = {'Alabama':'AL','Alaska':'AK','Arizona':'AZ','Arkansas':'AR','California':'CA','Colorado':'CO','Connecticut':'CT','Delaware':'DE','Florida':'FL','Georgia':'GA','Hawaii':'HI','Idaho':'ID','Illinois':'IL','Indiana':'IN','Iowa':'IA','Kansas':'KS','Kentucky':'KY','Louisiana':'LA','Maine':'ME','Maryland':'MD','Massachusetts':'MA','Michigan':'MI','Minnesota':'MN','Mississippi':'MS','Missouri':'MO','Montana':'MT','Nebraska':'NE','Nevada':'NV','New Hampshire':'NH','New Jersey':'NJ','New Mexico':'NM','New York':'NY','North Carolina':'NC','North Dakota':'ND','Ohio':'OH','Oklahoma':'OK','Oregon':'OR','Pennsylvania':'PA','Rhode Island':'RI','South Carolina':'SC','South Dakota':'SD','Tennessee':'TN','Texas':'TX','Utah':'UT','Vermont':'VT','Virginia':'VA','Washington':'WA','West Virginia':'WV','Wisconsin':'WI','Wyoming':'WY'}
out = {}
maxx = maxy = 0
for g in T["objects"]["states"]["geometries"]:
    n = g["properties"]["name"]
    if n not in AB: continue
    polys = [g["arcs"]] if g["type"] == "Polygon" else g["arcs"]
    rs = [[ring(r) for r in p] for p in polys]
    d = "".join("M" + "L".join(f"{x:.1f},{y:.1f}" for x, y in r) + "Z" for p in rs for r in p)
    b = max(rs, key=lambda p: len(p[0]))[0]
    xs = [p[0] for p in b]; ys = [p[1] for p in b]
    maxx = max(maxx, max(p[0] for p in sum((r for p in rs for r in p), [])))
    maxy = max(maxy, max(p[1] for p in sum((r for p in rs for r in p), [])))
    out[AB[n]] = {"name": n, "d": d, "cx": round((min(xs) + max(xs)) / 2, 1), "cy": round((min(ys) + max(ys)) / 2, 1)}
assert len(out) == 50
json.dump({"width": round(maxx + 4), "height": round(maxy + 4), "states": out}, open(os.path.join(here, "..", "data", "map.json"), "w"), separators=(",", ":"))
print("ok", len(out), maxx, maxy)
