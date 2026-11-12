#!/usr/bin/env python3
"""show.py <outdir> [name...]: per (program, config, linker), the labelled lines that differ from native -O."""
import json, sys
out = sys.argv[1]
names = sys.argv[2:]
res = json.load(open(out + "/results.json"))
for name in sorted(res):
    if names and name not in names:
        continue
    nat = res[name]["native"]
    if "error" in nat:
        print(name, "NATIVE ERROR", nat["error"][-800:])
        continue
    if not nat["agree"]:
        print(name, "NATIVE O0/O3 DIFFER")
    nl = nat["O"].splitlines()
    for cfg in sorted(k for k in res[name] if k != "native"):
        for linker, r in sorted(res[name][cfg].items()):
            if r["status"] != "RAN":
                errs = " | ".join(l for l in r["detail"].splitlines() if l.startswith("error"))[:600]
                print(f"{name} [{cfg}/{linker}] COMPILE_FAIL: {errs}")
                continue
            ml = r["stdout"].splitlines()
            if ml == nl:
                continue
            bad = []
            for i in range(max(len(nl), len(ml))):
                a = nl[i] if i < len(nl) else "<none>"
                b = ml[i] if i < len(ml) else "<none>"
                if a != b:
                    label = nl[i - 1] if i > 0 and i - 1 < len(nl) else "?"
                    bad.append(f"{label}: native={a} mires={b}")
            print(f"{name} [{cfg}/{linker}] {len(bad)} differing lines")
            for b in bad[:40]:
                print("    " + b)
