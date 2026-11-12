#!/usr/bin/env python3
"""fxlanes.py <fx outdir> <config>...: over every fixture, base against fix link lane reports:
items whose lane changed, items whose in-loop allocation counts changed, representation commits,
flat-ABI set changes, whole-program allocation totals."""
import json, os, sys

out = sys.argv[1]
for c in sys.argv[2:]:
    lane_changes, loop_changes, repr_changes, abi_changes, totals, fallbacks = [], [], [], [], [], []
    n_fx = 0
    for n in sorted(os.listdir(os.path.join(out, "fix"))):
        pb = os.path.join(out, "base", n, c, "out.js.lane.json")
        pf = os.path.join(out, "fix", n, c, "out.js.lane.json")
        if not (os.path.exists(pb) and os.path.exists(pf)):
            continue
        n_fx += 1
        b, f = json.load(open(pb)), json.load(open(pf))
        bi = {i["name"]: i for i in b["items"]}
        fi = {i["name"]: i for i in f["items"]}
        for k in sorted(set(bi) & set(fi)):
            x, y = bi[k], fi[k]
            if x["lane"] != y["lane"]:
                lane_changes.append((n, k, x["lane"], x["decline"], y["lane"], y["decline"]))
            lx = ((x.get("alloc") or {}).get("in_loop"), (x.get("js_alloc") or {}).get("in_loop"))
            ly = ((y.get("alloc") or {}).get("in_loop"), (y.get("js_alloc") or {}).get("in_loop"))
            if lx != ly and x["lane"] == y["lane"] and k != "rust_entry":
                loop_changes.append((n, k, lx, ly))
        for i in f["items"]:
            if i["lane"] != "jir":
                fallbacks.append((n, i["name"], i["decline"]))
        cb = [d["subject"] for d in b["repr"]["decisions"] if d["chosen"] != "baseline"]
        cf = [d["subject"] for d in f["repr"]["decisions"] if d["chosen"] != "baseline"]
        if cb != cf:
            repr_changes.append((n, sorted(set(cb) - set(cf)), sorted(set(cf) - set(cb))))
        fb = sorted(x["item"] for x in b["abi"]["flat"])
        ff = sorted(x["item"] for x in f["abi"]["flat"])
        if fb != ff:
            abi_changes.append((n, len(fb), len(ff)))
        pbv, pfv = (b["physical"].get("validated") or b["physical"].get("canonical") or {}), (f["physical"].get("validated") or f["physical"].get("canonical") or {})
        if (pbv.get("allocations"), pbv.get("loop_allocations")) != (pfv.get("allocations"), pfv.get("loop_allocations")):
            totals.append((n, (pbv.get("allocations"), pbv.get("loop_allocations")), (pfv.get("allocations"), pfv.get("loop_allocations"))))
    print(f"== [{c}] fixtures with lane reports on both: {n_fx}")
    print(f"  items whose lane changed: {len(lane_changes)}")
    for r in lane_changes[:40]:
        print("    ", r)
    print(f"  fallback items with the fix: {len(fallbacks)}")
    for r in fallbacks[:40]:
        print("    ", r)
    print(f"  items (other than rust_entry) whose in-loop allocation count changed: {len(loop_changes)}")
    for r in loop_changes[:60]:
        print("    ", r)
    print(f"  representation commits changed: {repr_changes}")
    print(f"  flat-ABI set changed: {abi_changes}")
    print(f"  whole-program validated (allocations, loop_allocations) changed: {len(totals)}")
    for r in totals[:60]:
        print("    ", r)
