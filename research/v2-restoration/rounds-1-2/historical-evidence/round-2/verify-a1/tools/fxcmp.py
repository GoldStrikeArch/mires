#!/usr/bin/env python3
"""fxcmp.py <fx/all results.json> <variant results.json> <variant linker name> <config>...:
for a variant linker, per configuration: fixtures whose status differs from the fix, and the byte
totals of base / fix / variant over fixtures all three compiled, with how many fixtures differ
from base."""
import json, sys

allr = json.load(open(sys.argv[1]))
var = json.load(open(sys.argv[2]))
name = sys.argv[3]
for c in sys.argv[4:]:
    tb = tf = tv = 0
    diff_status, changed_fix, changed_var = [], 0, 0
    for n in sorted(var):
        if n not in allr or c not in var[n] or c not in allr[n]:
            continue
        b, f, v = allr[n][c]["base"], allr[n][c]["fix"], var[n][c][name]
        if v["status"] != f["status"]:
            diff_status.append((n, f["status"], v["status"]))
        if n[:3] in ("210", "211", "212", "213", "214"):
            continue
        if b["bytes"] and f["bytes"] and v["bytes"]:
            tb += b["bytes"]; tf += f["bytes"]; tv += v["bytes"]
            changed_fix += b["sha"] != f["sha"]
            changed_var += b["sha"] != v["sha"]
    print(f"[{c}] {name}: status differs from fix: {diff_status}")
    print(f"      bytes over existing fixtures: base {tb}, fix {tf} ({tf - tb:+d}, {changed_fix} fixtures changed), "
          f"{name} {tv} ({tv - tb:+d}, {changed_var} fixtures changed)")
