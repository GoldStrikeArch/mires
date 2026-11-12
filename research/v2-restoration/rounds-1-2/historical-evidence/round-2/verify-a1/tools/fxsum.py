#!/usr/bin/env python3
"""fxsum.py <fx outdir>: base against fix over the fixture matrix: status regressions, cells where
both fail (and whether their stdout agrees), changed-output counts per configuration, and for
every fixture the lane-report differences (lane, loop allocations)."""
import json, os, sys

out = sys.argv[1]
res = json.load(open(out + "/results.json"))
configs = sorted({c for n in res for c in res[n]})
reg, bothfail, fixed = [], [], []
changed = {c: [] for c in configs}
for n in sorted(res):
    for c in configs:
        b, f = res[n][c]["base"], res[n][c]["fix"]
        if b["status"] == "PASS" and f["status"] != "PASS":
            reg.append((n, c, f["status"]))
        if b["status"] != "PASS" and f["status"] == "PASS":
            fixed.append((n, c))
        if b["status"] != "PASS" and f["status"] != "PASS":
            def rd(l):
                p = os.path.join(out, l, n, c, "stdout")
                return open(p).read() if os.path.exists(p) else None
            bothfail.append((n, c, b["status"], f["status"], rd("base") == rd("fix")))
        if b["sha"] != f["sha"]:
            changed[c].append((n, f["bytes"] - b["bytes"]))
print("REGRESSIONS (PASS on main, not PASS with the fix):", reg)
print("fixed by the change:", len(fixed), sorted({n for n, _ in fixed}))
print("fail on both:", len(bothfail), "of which stdout differs between base and fix:",
      [(n, c) for n, c, _, _, same in bothfail if not same])
print("  fixtures failing on both:", sorted({n for n, *_ in bothfail}))
for c in configs:
    ch = changed[c]
    print(f"[{c}] fixtures with changed JS: {len(ch)}; net bytes {sum(d for _, d in ch):+d}; smaller: {[x for x in ch if x[1] < 0]}")
