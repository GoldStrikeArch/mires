#!/usr/bin/env python3
"""itemstat.py <fx outdir> <fixture> <name substring> <config>...: lane-report rows of the matching
items for base and fix."""
import json, sys

out, fixture, sub = sys.argv[1:4]
for c in sys.argv[4:]:
    for l in ("base", "fix"):
        d = json.load(open(f"{out}/{l}/{fixture}/{c}/out.js.lane.json"))
        for i in d["items"]:
            if sub in i["name"]:
                js = i["js_alloc"] and {k: i["js_alloc"][k] for k in ("object", "array", "closure", "new", "in_loop")}
                print(c, l, i["name"], i["lane"], "bytes", i["bytes_min"], "jir in_loop", i["alloc"]["in_loop"], "js", js)
