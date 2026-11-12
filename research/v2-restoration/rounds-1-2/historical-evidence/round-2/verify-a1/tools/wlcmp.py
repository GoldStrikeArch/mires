#!/usr/bin/env python3
"""wlcmp.py <base wl dir> <fix wl dir>: per workload, minified bytes, link-lane fallback counts,
flat-ABI set sizes, representation commits and validated allocation totals, base against fix."""
import json, os, sys

b, f = sys.argv[1], sys.argv[2]
tb = tf = 0
for w in sorted(os.listdir(os.path.join(f, "dist"))):
    pb = os.path.join(b, "dist", w, "index.min.js")
    pf = os.path.join(f, "dist", w, "index.min.js")
    if not (os.path.exists(pb) and os.path.exists(pf)):
        print(w, "missing on one side")
        continue
    sb, sf = os.path.getsize(pb), os.path.getsize(pf)
    tb += sb; tf += sf
    lb = json.load(open(os.path.join(b, "_build", w, "program.js.lane.json")))
    lf = json.load(open(os.path.join(f, "_build", w, "program.js.lane.json")))
    def facts(l):
        ph = l["physical"].get("validated") or l["physical"].get("canonical") or {}
        return (l["fallback_count"], len(l["abi"]["flat"]),
                len([d for d in l["repr"]["decisions"] if d["chosen"] != "baseline"]),
                ph.get("allocations"), ph.get("loop_allocations"))
    print(f"{w:15s} min bytes {sb} -> {sf} ({sf - sb:+d})  (fallbacks, flat ABI items, repr commits, allocations, loop allocations) {facts(lb)} -> {facts(lf)}")
print(f"total {tb} -> {tf} ({tf - tb:+d})")
