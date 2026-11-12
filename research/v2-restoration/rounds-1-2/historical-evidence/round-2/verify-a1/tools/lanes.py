#!/usr/bin/env python3
"""lanes.py <base lane.json> <fix lane.json> [label]: per item differences between two link lane
reports: lane (jir / fallback) and decline reason, allocations inside loops, allocation counts."""
import json, sys


def load(p):
    d = json.load(open(p))
    return d, {i["name"]: i for i in d["items"]}


def loops(i):
    a = (i.get("alloc") or {}).get("in_loop", 0)
    j = (i.get("js_alloc") or {}).get("in_loop", 0) if i.get("js_alloc") else None
    return a, j


def allocs(i):
    j = i.get("js_alloc")
    if not j:
        return None
    return j["object"] + j["array"] + j["closure"] + j["new"]


bd, b = load(sys.argv[1])
fd, f = load(sys.argv[2])
label = sys.argv[3] if len(sys.argv) > 3 else ""
print(f"{label} items base {len(b)} fix {len(f)}; fallback_count base {bd['fallback_count']} fix {fd['fallback_count']}")
for name in sorted(set(b) | set(f)):
    x, y = b.get(name), f.get(name)
    if x is None or y is None:
        print(f"  ONLY-{'FIX' if x is None else 'BASE'} {name} lane={(y or x)['lane']} decline={(y or x)['decline']}")
        continue
    if x["lane"] != y["lane"] or x["decline"] != y["decline"]:
        print(f"  LANE {name}: {x['lane']}({x['decline']}) -> {y['lane']}({y['decline']})")
    if loops(x) != loops(y):
        print(f"  LOOP-ALLOC {name}: (jir, js) {loops(x)} -> {loops(y)}")
    ax, ay = allocs(x), allocs(y)
    if ax is not None and ay is not None and ax != ay:
        print(f"  ALLOC {name}: js allocation sites {ax} -> {ay}  bytes {x['bytes_min']} -> {y['bytes_min']}")
fb = [i for i in fd["items"] if i["lane"] != "jir"]
if fb:
    print("  fallback items with the fix:")
    for i in fb:
        print(f"    {i['name']}: {i['decline']} ({i['bytes_baseline']} B baseline text)")
