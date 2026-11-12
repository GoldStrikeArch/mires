#!/usr/bin/env python3
"""reprcmp.py <base lane.json> <fix lane.json>: representation decisions that differ."""
import json, sys

b = json.load(open(sys.argv[1]))["repr"]
f = json.load(open(sys.argv[2]))["repr"]
for k in b:
    if k != "decisions" and b[k] != f.get(k):
        print(k, "BASE", json.dumps(b[k])[:400])
        print(k, "FIX ", json.dumps(f.get(k))[:400])
db = {d["subject"] + "|" + d["family"]: d for d in b["decisions"]}
df = {d["subject"] + "|" + d["family"]: d for d in f["decisions"]}
for k in sorted(set(db) | set(df)):
    x, y = db.get(k), df.get(k)
    if x is None or y is None:
        print("ONLY", "FIX" if x is None else "BASE", json.dumps(y or x)[:300])
    elif x != y:
        print("DIFF", k)
        print("   base", json.dumps(x)[:400])
        print("   fix ", json.dumps(y)[:400])
print("decisions base", len(db), "fix", len(df), "committed base",
      [k for k, d in db.items() if d["chosen"] != "baseline"][:10], "fix",
      [k for k, d in df.items() if d["chosen"] != "baseline"][:10])
