#!/usr/bin/env python3
"""table.py <tag> [--msg]: one row per program, one column per compiler/mode, from out/<tag>/summary.txt.
P = matches native, W = ran to completion with different output (silent wrong), C = crashed at run
time, R = refused at compile time. With --msg, prints the first error line of each R/C cell."""
import re, sys, collections
from pathlib import Path
V = Path("/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a4-r3")
tag = sys.argv[1]
msg = "--msg" in sys.argv
rows = collections.OrderedDict()
cols = []
for line in (V / "out" / tag / "summary.txt").read_text().splitlines():
    parts = line.split()
    if len(parts) < 4 or parts[1] not in ("fix", "main"):
        print("??", line[:120]); continue
    name, comp, mode, status = parts[0], parts[1], parts[2], parts[3]
    col = f"{comp}:{mode}"
    if col not in cols: cols.append(col)
    if status == "PASS": cell = "P"
    else:
        m = re.search(r"exit=(\S+)", line)
        ex = m.group(1) if m else "?"
        cell = "R" if ex == "compile" else ("W" if ex == "0" else "C")
    rows.setdefault(name, {})[col] = cell
order = [f"{c}:{m}" for c in ("fix", "main") for m in ("def", "jiroff", "rb", "rbz")]
cols = [c for c in order if c in cols]
print("program".ljust(28), " ".join(c.replace("jiroff", "jo") for c in cols))
for name in sorted(rows):
    print(name.ljust(28), " ".join(rows[name].get(c, "-").center(len(c.replace("jiroff", "jo"))) for c in cols))
if msg:
    for name in sorted(rows):
        for c in cols:
            if rows[name].get(c) in ("R", "C"):
                comp, mode = c.split(":")
                base = V / "out" / tag / comp / f"{name}.{mode}"
                text = ""
                if rows[name][c] == "R" and base.with_suffix(f".{mode}.log").exists():
                    log = (V / "out" / tag / comp / f"{name}.{mode}.log").read_text()
                    errs = [l for l in log.splitlines() if l.startswith("error")]
                    text = (errs[0] if errs else log[:200])
                    text = re.sub(r"/private/tmp/\S*/src/", "src/", text)
                else:
                    err = (V / "out" / tag / comp / f"{name}.{mode}.stderr").read_text()
                    errs = [l for l in err.splitlines() if "Error" in l]
                    text = errs[0] if errs else err[:200]
                print(f"  {name} {c}: {text[:230]}")
