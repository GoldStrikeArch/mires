#!/usr/bin/env python3
"""fx.py <outdir> --linkers name=path,... --configs rb,rbz,... [-j N] [--dir DIR] [fixture-prefix...]

Compiles fixtures (default: examples/core-tests/*.rs that have a .expected) with each linker in
each configuration, runs them, and compares stdout with the .expected file. Prints one line per
(fixture, config) that is not all-PASS, then totals; writes <outdir>/results.json (status, bytes,
sha256 of the emitted JS).
"""
import concurrent.futures as cf
import hashlib, json, os, subprocess, sys

W = "/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ae6e006621ef540c6"
CONFIGS = {
    "rb": ("js-profile=release-balanced", {}),
    "rbmin": ("js-profile=release-balanced js-minify=on", {}),
    "def": ("", {}),
    "jiron": ("js-lane=jir js-opt=on js-cleanup=on", {}),
    "jiroff": ("js-lane=jir js-opt=off", {}),
    "rbz": ("js-profile=release-balanced", {"JSC_RUST_OPT_LEVEL": "z", "JSC_RUST_MIR_INLINE": "on"}),
    "defz": ("", {"JSC_RUST_OPT_LEVEL": "z", "JSC_RUST_MIR_INLINE": "on"}),
    "jironz": ("js-lane=jir js-opt=on js-cleanup=on", {"JSC_RUST_OPT_LEVEL": "z", "JSC_RUST_MIR_INLINE": "on"}),
    "jiroffz": ("js-lane=jir js-opt=off", {"JSC_RUST_OPT_LEVEL": "z", "JSC_RUST_MIR_INLINE": "on"}),
}


def sh(cmd, env=None, timeout=600, stdin=None):
    e = dict(os.environ)
    if env:
        e.update(env)
    try:
        p = subprocess.run(cmd, env=e, capture_output=True, timeout=timeout, input=stdin)
        return p.returncode, p.stdout.decode("utf8", "replace"), p.stderr.decode("utf8", "replace")
    except subprocess.TimeoutExpired:
        return 124, "", "TIMEOUT"


def run(src, expected, od, linker, config):
    os.makedirs(od, exist_ok=True)
    out = os.path.join(od, "out.js")
    args, extra = CONFIGS[config]
    env = {"JSC_LINK_BIN": linker, "JS_EXTRA_ARGS": args, "RCGJS_INTEGRATION_LOCK_HELD": W + "/build/.integration-lock"}
    env.update(extra)
    rc, so, se = sh(["bash", W + "/scripts/compile_core.sh", src, out], env=env)
    if rc != 0 or not os.path.exists(out):
        open(os.path.join(od, "compile.log"), "w").write(so + se)
        return {"status": "COMPILE_FAIL", "bytes": 0, "sha": ""}
    text = open(out).read()
    js = open(W + "/runtime/shim.js").read() + text + open(W + "/scripts/entry.js").read()
    rc, so, se = sh(["node", "-"], stdin=js.encode(), timeout=120)
    open(os.path.join(od, "stdout"), "w").write(so + (f"\n[node exit {rc}] {se[-400:]}" if rc else ""))
    ok = so == expected
    return {"status": "PASS" if ok else ("DIFF" if rc == 0 else "ABORT_DIFF"), "bytes": len(text.encode()),
            "sha": hashlib.sha256(text.encode()).hexdigest()[:16], "rc": rc}


def main():
    args = sys.argv[1:]
    outdir = args[0]
    args = args[1:]
    linkers, configs, jobs, d, prefixes = {}, ["rb"], 5, W + "/examples/core-tests", []
    i = 0
    while i < len(args):
        if args[i] == "--linkers":
            for kv in args[i + 1].split(","):
                k, v = kv.split("=", 1)
                linkers[k] = v
            i += 2
        elif args[i] == "--configs":
            configs = args[i + 1].split(","); i += 2
        elif args[i] == "-j":
            jobs = int(args[i + 1]); i += 2
        elif args[i] == "--dir":
            d = args[i + 1]; i += 2
        else:
            prefixes.append(args[i]); i += 1
    names = sorted(f[:-3] for f in os.listdir(d) if f.endswith(".rs") and os.path.exists(os.path.join(d, f[:-3] + ".expected")))
    if prefixes:
        names = [n for n in names if any(n.startswith(p) for p in prefixes)]
    results = {}
    with cf.ThreadPoolExecutor(jobs) as pool:
        futs = {}
        for n in names:
            exp = open(os.path.join(d, n + ".expected")).read()
            for c in configs:
                for l, path in linkers.items():
                    futs[pool.submit(run, os.path.join(d, n + ".rs"), exp, os.path.join(outdir, l, n, c), path, c)] = (n, c, l)
        for fut in cf.as_completed(futs):
            n, c, l = futs[fut]
            results.setdefault(n, {}).setdefault(c, {})[l] = fut.result()
    json.dump(results, open(os.path.join(outdir, "results.json"), "w"), indent=1)
    tot = {(c, l): [0, 0] for c in configs for l in linkers}
    for n in names:
        for c in configs:
            cells = []
            allpass = True
            for l in linkers:
                r = results[n][c][l]
                tot[(c, l)][1] += 1
                if r["status"] == "PASS":
                    tot[(c, l)][0] += 1
                else:
                    allpass = False
                cells.append(f"{l}={r['status']}({r['bytes']})")
            if not allpass:
                print(f"{n} [{c}]: " + " ".join(cells))
    for (c, l), (p, t) in sorted(tot.items()):
        print(f"TOTAL [{c}] {l}: {p}/{t}")


if __name__ == "__main__":
    main()
