#!/usr/bin/env python3
"""lanes.py <dirA> <dirB>: per *.js.lane.json present in both directories, the number of items that
left the JIR lane (fallback_count) in A and in B, printing only files where they differ, then
totals. Used to tell a surviving mutant that is equivalent from one that only moved items to the
baseline lane, and to compare main with the fix."""
import json, sys
from pathlib import Path
a, b = Path(sys.argv[1]), Path(sys.argv[2])
ta = tb = n = 0
for fa in sorted(a.glob("*.js.lane.json")):
    fb = b / fa.name
    if not fb.exists():
        continue
    ca = json.load(open(fa))["fallback_count"]
    cb = json.load(open(fb))["fallback_count"]
    ta += ca; tb += cb; n += 1
    if ca != cb:
        print(f"  {fa.name}: {ca} -> {cb}")
print(f"files compared {n}; off-lane items: {a.name}={ta} {b.name}={tb}")
