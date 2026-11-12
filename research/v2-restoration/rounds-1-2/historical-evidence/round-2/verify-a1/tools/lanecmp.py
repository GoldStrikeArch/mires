#!/usr/bin/env python3
"""lanecmp.py <base lane.json> <fix lane.json>: optimizer decisions that differ between two link
lane reports (pass counts, inlining, flat ABI, per item bytes)."""
import json, sys

b = json.load(open(sys.argv[1]))
f = json.load(open(sys.argv[2]))
for k in ("passes", "inline", "devirt", "range", "plan_retries", "repr", "slice_iter", "async"):
    if b.get(k) != f.get(k):
        print(k, "BASE", json.dumps(b.get(k))[:1200])
        print(k, "FIX ", json.dumps(f.get(k))[:1200])
print("abi flat base", len(b["abi"]["flat"]), "fix", len(f["abi"]["flat"]))
sb = set(json.dumps(x) for x in b["abi"]["flat"])
sf = set(json.dumps(x) for x in f["abi"]["flat"])
print("flat only base", list(sb - sf)[:8])
print("flat only fix", list(sf - sb)[:8])
db = {x["item"]: x["reason"] for x in b["abi"]["declined"]}
df = {x["item"]: x["reason"] for x in f["abi"]["declined"]}
for k in sorted(set(db) | set(df)):
    if db.get(k) != df.get(k):
        print("abi declined", k, db.get(k), "->", df.get(k))
print("physical base", json.dumps(b["physical"])[:500])
print("physical fix ", json.dumps(f["physical"])[:500])
bi = {i["name"]: i for i in b["items"]}
fi = {i["name"]: i for i in f["items"]}
tot = 0
rows = []
for n in sorted(set(bi) & set(fi)):
    d = fi[n]["bytes_min"] - bi[n]["bytes_min"]
    if d:
        rows.append((d, n, bi[n]["bytes_min"], fi[n]["bytes_min"]))
        tot += d
for d, n, x, y in sorted(rows):
    print(f"  {d:+6d} {n} {x}->{y}")
print("items changed", len(rows), "sum over common items", tot)
print("only base:", [(n, bi[n]["bytes_min"]) for n in sorted(set(bi) - set(fi))])
print("only fix:", [(n, fi[n]["bytes_min"]) for n in sorted(set(fi) - set(bi))])
