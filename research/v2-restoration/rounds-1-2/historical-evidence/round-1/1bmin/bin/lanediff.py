#!/usr/bin/env python3
"""lanediff.py <a.lane.json> <b.lane.json>: per-item differences between two lane reports."""
import json, sys
a = json.load(open(sys.argv[1])); b = json.load(open(sys.argv[2]))
print('keys', list(a.keys()))
def rows(d):
    for key in ('items', 'rows', 'offered'):
        if isinstance(d.get(key), list):
            return {str(r.get('item', r.get('name'))): r for r in d[key]}
    return {}
ia, ib = rows(a), rows(b)
print('rows', len(ia), len(ib))
n = 0
for k in sorted(set(ia) | set(ib)):
    ra, rb = ia.get(k), ib.get(k)
    if ra is None or rb is None:
        print('ONLY', 'base' if rb is None else 'mine', k); n += 1; continue
    keys = set(ra) | set(rb)
    diffs = {x: (ra.get(x), rb.get(x)) for x in keys if ra.get(x) != rb.get(x)}
    if diffs:
        n += 1
        if n <= 40:
            print(k, diffs)
print('differing rows', n)
