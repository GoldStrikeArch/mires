#!/usr/bin/env python3
"""probe.py <jobs file> <results.tsv> [workers]

One job per line: `id|keepdir|outdir|expected-or-dash|calls-or-dash|link args`.
For each job the kept program (tools/cc.sh) is relinked with the cf80163 linker (base) and the
linker under test (fix) into <outdir>/{base,fix}; every accepted link is loaded in every file order
(tools/orders.mjs). One TSV row per job:

  id  class  base-outcome  fix-outcome  same-bytes  detail

Outcomes: REFUSED | OK(n orders) | LOADFAIL(k/n orders; first error) | OPTS (the option parser said no).
Classes:
  SAME-OK            both accept, every order runs and prints the expectation
  BOTH-REFUSED       both refuse
  FAIL-TO-REFUSAL    base accepted a program that fails under some load order; fix refuses it
  FALSE-REFUSAL      base accepted, every load order ran right; fix refuses it
  FALSE-ACCEPT       fix accepted a program that fails under some load order
  UNREFUSED          base refused, fix accepts and it runs (a refusal went away)
  MESSAGE            fix refused with a panic / internal error / no `error:` line
"""
import json, os, subprocess, sys, filecmp
from concurrent.futures import ThreadPoolExecutor

V = "/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6b"
W = "/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-acddce572a795adec"
SHIM = W + "/build/esm/shim.js"


def link(keep, which, out, args):
    subprocess.run(["bash", V + "/tools/ln.sh", keep, which, out] + args, check=False)
    status = open(out + "/status").read().strip()
    log = open(out + "/link.log", errors="replace").read()
    return status, log


def load(out, expected, calls):
    r = subprocess.run(["node", V + "/tools/orders.mjs", out, SHIM, expected, calls],
                       capture_output=True, text=True)
    try:
        return json.loads(r.stdout.strip().splitlines()[-1])
    except Exception:
        return {"files": [], "orders": 0, "ok": 0, "bad": [{"order": "?", "status": -1, "error": "orders.mjs: " + (r.stderr or r.stdout)[:200]}]}


def side(keep, which, out, args, expected, calls):
    status, log = link(keep, which, out, args)
    if status == "opts":
        return "OPTS", log, None
    if status != "0":
        return "REFUSED", log, None
    res = load(out, expected, calls)
    if res["bad"]:
        firsts = sorted({b["order"].split(">")[0] for b in res["bad"]})
        return "LOADFAIL", log, res
    return "OK", log, res


def describe(kind, log, res):
    if kind in ("REFUSED", "OPTS"):
        first = next((l for l in log.splitlines() if l.startswith("error")), log.strip().splitlines()[0] if log.strip() else "")
        return f"{kind}: {first[:230]}"
    if kind == "OK":
        return f"OK({res['orders']} orders of {','.join(res['files'])})"
    firsts = sorted({b["order"].split(">")[0] for b in res["bad"]})
    return f"LOADFAIL({len(res['bad'])}/{res['orders']} orders; first-loaded in failing orders: {','.join(firsts)}; {res['bad'][0]['error'][:120]})"


def job(line):
    ident, keep, out, expected, calls, args = line.rstrip("\n").split("|")
    args = args.split()
    b = side(keep, "base", out + "/base", args, expected, calls)
    fstatus, flog = link(keep, os.environ.get("PROBE_FIX", "fix"), out + "/fix", args)
    same = "-"
    if b[0] in ("OK", "LOADFAIL") and fstatus == "0":
        names = sorted(n for n in os.listdir(out + "/base") if n.endswith(".js") or n.endswith(".json"))
        names2 = sorted(n for n in os.listdir(out + "/fix") if n.endswith(".js") or n.endswith(".json"))
        same = "same" if names == names2 and all(filecmp.cmp(f"{out}/base/{n}", f"{out}/fix/{n}", shallow=False) for n in names) else "DIFFER"
    if same == "same":
        # Byte-identical files load identically: the base result stands for both.
        f = (b[0], flog, b[2])
    elif fstatus == "opts":
        f = ("OPTS", flog, None)
    elif fstatus != "0":
        f = ("REFUSED", flog, None)
    else:
        res = load(out + "/fix", expected, calls)
        f = ("LOADFAIL" if res["bad"] else "OK", flog, res)
    if f[0] == "LOADFAIL":
        cls = "FALSE-ACCEPT"
    elif f[0] == "OK":
        cls = "SAME-OK" if b[0] == "OK" else ("UNREFUSED" if b[0] == "REFUSED" else "BASE-" + b[0])
    elif f[0] == "REFUSED":
        cls = {"REFUSED": "BOTH-REFUSED", "OK": "FALSE-REFUSAL", "LOADFAIL": "FAIL-TO-REFUSAL"}.get(b[0], "BASE-" + b[0])
        log = f[1]
        lines = [l for l in log.splitlines() if l.strip()]
        if "panicked" in log or "internal error" in log.lower() or "RUST_BACKTRACE" in log or not lines or not all(l.startswith(("error", "warning")) for l in lines):
            cls = "MESSAGE/" + cls
    else:
        cls = "OPTS"
    return "\t".join([ident, cls, describe(*b), describe(*f), same])


def main():
    jobs = [l for l in open(sys.argv[1]) if l.strip() and not l.startswith("#")]
    workers = int(sys.argv[3]) if len(sys.argv) > 3 else 6
    with ThreadPoolExecutor(workers) as pool, open(sys.argv[2], "w") as out:
        for row in pool.map(job, jobs):
            out.write(row + "\n")
            out.flush()
    counts = {}
    for row in open(sys.argv[2]):
        c = row.split("\t")[1]
        counts[c] = counts.get(c, 0) + 1
    print(" ".join(f"{k}={v}" for k, v in sorted(counts.items())))


main()
