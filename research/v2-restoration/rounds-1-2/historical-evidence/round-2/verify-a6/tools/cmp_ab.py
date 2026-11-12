#!/usr/bin/env python3
"""cmp_ab.py <ab/tag>: byte-compares every program test.sh left under base/ and fix/.

Prints the number identical, the files that differ, the files only one side has, and the rows of
the two test.sh tables that differ (PASS/FAIL and byte counts)."""
import os, re, sys, filecmp

root = sys.argv[1]
same, differ, only = 0, [], []
for sub in ("", "core"):
    b, f = os.path.join(root, "base", sub), os.path.join(root, "fix", sub)
    names = set()
    for d in (b, f):
        if os.path.isdir(d):
            names |= {n for n in os.listdir(d) if n.endswith(".js")}
    for n in sorted(names):
        pb, pf = os.path.join(b, n), os.path.join(f, n)
        if not (os.path.exists(pb) and os.path.exists(pf)):
            only.append((sub, n, "base" if os.path.exists(pb) else "fix"))
        elif filecmp.cmp(pb, pf, shallow=False):
            same += 1
        else:
            differ.append((sub, n))
print(f"{root}: identical {same}, differ {len(differ)}, one-sided {len(only)}")
for d in differ:
    print("  DIFFER", d)
for o in only:
    print("  ONLY", o)

def rows(path):
    out = {}
    for line in open(path, errors="replace"):
        m = re.match(r"^(\S+)\s+(PASS|FAIL)\s+(.*)$", line.rstrip())
        if m:
            out[m.group(1)] = (m.group(2), re.sub(r"\s+", " ", m.group(3)))
    return out

rb, rf = rows(os.path.join(root, "base", "test.log")), rows(os.path.join(root, "fix", "test.log"))
for k in sorted(set(rb) | set(rf)):
    if rb.get(k) != rf.get(k):
        print("  ROW", k, "base:", rb.get(k), "fix:", rf.get(k))
print("  base:", open(os.path.join(root, "base", "test.log"), errors="replace").read().strip().splitlines()[-3:])
print("  fix: ", open(os.path.join(root, "fix", "test.log"), errors="replace").read().strip().splitlines()[-3:])
