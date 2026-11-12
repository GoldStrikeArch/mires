#!/usr/bin/env python3
"""probe.py <srcdir> <outdir> [--configs rb,def,jiron,jiroff,basen] [--linkers base,fix] [-j N] [names...]

Compiles every <srcdir>/*.rs (except prelude.rs) natively (-O and -C opt-level=0) and through
mires with each linker under each configuration, runs everything, and compares stdout.
Writes <outdir>/results.json and prints one line per (program, config).
"""
import concurrent.futures as cf
import json, os, re, subprocess, sys

W = "/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ae6e006621ef540c6"
S = "/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a1"
LINKERS = {
    "base": S + "/base-src/target/release/jsc-link",
    "fix": os.environ.get("PROBE_FIX_LINKER", W + "/target/release/jsc-link"),
}
for _kv in filter(None, os.environ.get("PROBE_EXTRA", "").split(",")):
    LINKERS[_kv.split("=", 1)[0]] = _kv.split("=", 1)[1]
CONFIGS = {
    "rb": "js-profile=release-balanced",
    "def": "",
    "jiron": "js-lane=jir js-opt=on js-cleanup=on",
    "jiroff": "js-lane=jir js-opt=off",
    "basen": "js-lane=baseline js-opt=on js-cleanup=on",
    # The product build settings (experiments/*/tools/build.mjs): rustc -Copt-level=z and MIR inlining.
    "rbz": "js-profile=release-balanced",
    "rbmi": "js-profile=release-balanced",
    "jiroffmi": "js-lane=jir js-opt=off",
    "defz": "",
}
EXTRA_ENV = {
    "rbz": {"JSC_RUST_OPT_LEVEL": "z", "JSC_RUST_MIR_INLINE": "on"},
    "rbmi": {"JSC_RUST_MIR_INLINE": "on"},
    "jiroffmi": {"JSC_RUST_MIR_INLINE": "on"},
    "defz": {"JSC_RUST_OPT_LEVEL": "z", "JSC_RUST_MIR_INLINE": "on"},
}
NATIVE_PRELUDE = '''mod prelude {
    pub fn print_i32(x: i32) { println!("{}", x) }
    pub fn print_u32(x: u32) { println!("{}", x) }
    pub fn print_usize(x: usize) { println!("{}", x) }
    pub fn print_i64(x: i64) { println!("{}", x) }
    pub fn print_u64(x: u64) { println!("{}", x as i64) }
    pub fn print_f64(x: f64) { println!("{}", x) }
    pub fn print_bool(x: bool) { println!("{}", x) }
    pub fn print_str(s: &str) { println!("{}", s) }
}
use prelude::*;'''


def sh(cmd, env=None, timeout=600, stdin=None):
    e = dict(os.environ)
    if env:
        e.update(env)
    try:
        p = subprocess.run(cmd, env=e, capture_output=True, timeout=timeout, input=stdin)
        return p.returncode, p.stdout.decode("utf8", "replace"), p.stderr.decode("utf8", "replace")
    except subprocess.TimeoutExpired:
        return 124, "", "TIMEOUT"


def native(src, nd):
    os.makedirs(nd, exist_ok=True)
    text = open(src).read()
    text = text.replace("#![no_std]", "").replace("#![no_main]", "")
    text, n = re.subn(r'#\[path = "prelude.rs"\]\nmod prelude;\nuse prelude::\*;', lambda m: NATIVE_PRELUDE, text)
    assert n == 1, src
    text = text.replace("#[unsafe(no_mangle)]\npub fn rust_entry()", "pub fn rust_entry()")
    text += "\nfn main() { rust_entry(); }\n"
    main = os.path.join(nd, "main.rs")
    open(main, "w").write(text)
    outs = {}
    for tag, flags in (("O", ["-O"]), ("0", ["-C", "opt-level=0"])):
        exe = os.path.join(nd, "main" + tag)
        rc, _, err = sh(["rustc", "+nightly-2026-07-27", "--edition", "2021", "-A", "warnings", *flags, "-o", exe, main])
        if rc != 0:
            open(os.path.join(nd, f"build{tag}.log"), "w").write(err)
            return {"error": f"native {tag} build failed: " + err[-2000:]}
        rc, out, err = sh([exe], timeout=60)
        outs[tag] = out if rc == 0 else out + f"\n[native exit {rc}] {err[-300:]}"
        open(os.path.join(nd, f"{tag}.stdout"), "w").write(outs[tag])
    return {"O": outs["O"], "0": outs["0"], "agree": outs["O"] == outs["0"]}


def mires(src, od, linker, config):
    os.makedirs(od, exist_ok=True)
    out = os.path.join(od, "out.js")
    env = {
        "JSC_LINK_BIN": LINKERS[linker],
        "JS_EXTRA_ARGS": CONFIGS[config],
        # Read-only use of the keyed sysroot with output in scratch: do not queue behind suites.
        "RCGJS_INTEGRATION_LOCK_HELD": W + "/build/.integration-lock",
    }
    env.update(EXTRA_ENV.get(config, {}))
    rc, so, se = sh(["bash", W + "/scripts/compile_core.sh", src, out], env=env)
    if rc != 0 or not os.path.exists(out):
        open(os.path.join(od, "compile.log"), "w").write(so + se)
        return {"status": "COMPILE_FAIL", "detail": (so + se)[-1500:], "bytes": 0}
    js = open(W + "/runtime/shim.js").read() + open(out).read() + open(W + "/scripts/entry.js").read()
    rc, so, se = sh(["node", "-"], stdin=js.encode(), timeout=60)
    stdout = so if rc == 0 else so + f"\n[node exit {rc}] " + se.strip().splitlines()[0][:300] if se.strip() else so + f"\n[node exit {rc}]"
    open(os.path.join(od, "stdout"), "w").write(stdout)
    return {"status": "RAN", "stdout": stdout, "bytes": os.path.getsize(out), "rc": rc}


def main():
    args = sys.argv[1:]
    srcdir, outdir = args[0], args[1]
    args = args[2:]
    configs = ["rb", "def", "jiron", "jiroff", "basen"]
    linkers = list(LINKERS)
    jobs = 4
    names = []
    i = 0
    while i < len(args):
        if args[i] == "--configs":
            configs = args[i + 1].split(","); i += 2
        elif args[i] == "--linkers":
            linkers = args[i + 1].split(","); i += 2
        elif args[i] == "-j":
            jobs = int(args[i + 1]); i += 2
        else:
            names.append(args[i]); i += 1
    if not names:
        names = sorted(f[:-3] for f in os.listdir(srcdir) if f.endswith(".rs") and f != "prelude.rs")
    results = {}
    with cf.ThreadPoolExecutor(jobs) as pool:
        futs = {}
        for name in names:
            src = os.path.join(srcdir, name + ".rs")
            futs[pool.submit(native, src, os.path.join(outdir, "native", name))] = (name, "native", None)
            for c in configs:
                for l in linkers:
                    futs[pool.submit(mires, src, os.path.join(outdir, l, name, c), l, c)] = (name, c, l)
        for fut in cf.as_completed(futs):
            name, c, l = futs[fut]
            r = fut.result()
            results.setdefault(name, {})
            if c == "native":
                results[name]["native"] = r
            else:
                results[name].setdefault(c, {})[l] = r
    json.dump(results, open(os.path.join(outdir, "results.json"), "w"), indent=1)
    for name in names:
        nat = results[name]["native"]
        if "error" in nat:
            print(f"{name}: NATIVE ERROR {nat['error'][-600:]}")
            continue
        if not nat["agree"]:
            print(f"{name}: NATIVE -O and -O0 DIFFER")
        for c in configs:
            cells = []
            for l in linkers:
                r = results[name][c][l]
                if r["status"] == "COMPILE_FAIL":
                    cells.append(f"{l}=COMPILE_FAIL")
                else:
                    ok = r["stdout"] == nat["O"]
                    cells.append(f"{l}={'MATCH' if ok else 'DIFF'}({r['bytes']})")
            print(f"{name} [{c}]: " + " ".join(cells))


if __name__ == "__main__":
    main()
