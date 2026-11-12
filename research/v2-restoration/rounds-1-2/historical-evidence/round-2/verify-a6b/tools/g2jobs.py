#!/usr/bin/env python3
"""g2jobs.py <first seed> <last seed> <modes csv> <jobs file> [workers]

For each seed: generates src/g2_<seed>.rs (tools/gen2.py), runs it natively at -O0 and -O3
(verify-a6/tools/native.sh), compiles it once per mode keeping the link inputs (tools/cc.sh), and
writes the splits to try as job lines for tools/probe.py:
  * every entry function, in two orders (functions only);
  * each exported static alone with every function, the static first and last;
  * twelve random subsets of two to five candidates in random order, some with the registry,
    a renamed shared chunk and island roles;
  * every candidate at once.
"""
import os, random, subprocess, sys
from concurrent.futures import ThreadPoolExecutor

V = "/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6b"
A6 = "/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6"
first, last, modes, jobs_path = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3].split(","), sys.argv[4]
workers = int(sys.argv[5]) if len(sys.argv) > 5 else 6


def one(seed):
    heavy = bool(os.environ.get("G2_HEAVY"))
    name = ("g3_" if heavy else "g2_") + str(seed)
    src = f"{V}/src/{name}.rs"
    meta = f"{V}/src/{name}.meta"
    subprocess.run(["python3", f"{V}/tools/gen2.py", str(seed), src, meta], check=True)
    nat = subprocess.run(["bash", f"{A6}/tools/native.sh", src, f"{V}/native/{name}"], capture_output=True, text=True)
    if nat.returncode != 0:
        return [f"# {name} NATIVE-FAIL " + nat.stdout[:200].replace("\n", " ")]
    if open(f"{V}/native/{name}/exit-O3").read().strip() != "0":
        return [f"# {name} NATIVE-EXIT nonzero"]
    sections, cur = {}, None
    for line in open(nat.stdout.strip()):
        if line.startswith("== "):
            cur = int(line[3:]); sections[cur] = ""
        else:
            sections[cur] += line
    cands = [l.split() for l in open(meta)]
    fns = [n for k, n in cands if k == "fn"]
    statics = [n for k, n in cands if k == "static"]
    chunk = lambda n: ("e" + n[4:]) if n.startswith("ent_") else (n[0].lower() + n[2:])
    rng = random.Random(seed * 7919 + 1)
    splits = [(fns, ""), (fns[::-1], "")]
    for s in statics:
        splits.append(([s] + fns, ""))
        splits.append((fns + [s], ""))
    allc = fns + statics
    for _ in range(12):
        k = rng.randint(2, min(5, len(allc)))
        pick = rng.sample(allc, k)
        extra = ""
        r = rng.random()
        if r < 0.2:
            extra = "js-chunk-registry=on"
        elif r < 0.35:
            extra = "js-chunk-shared=common"
        elif r < 0.5:
            extra = " ".join(f"js-chunk-role={chunk(n)}:island" for n in pick)
        splits.append((pick, extra))
    if len(allc) <= 7:
        splits.append((allc, ""))
    if heavy:
        # Function entry points only, in every request order (thirty random ones past four).
        import itertools
        perms = [list(p) for p in itertools.permutations(fns)]
        if len(perms) > 24:
            perms = rng.sample(perms, 30)
        splits = [(p, "") for p in perms]
        roles = " ".join(f"js-chunk-role={chunk(n)}:island" for n in fns)
        splits += [(fns, "js-chunk-registry=on"), (fns, "js-chunk-shared=common"), (fns, roles),
                   (fns, "js-chunk-registry=on js-chunk-shared=common " + roles)]
        for leave in fns:
            splits.append(([f for f in fns if f != leave], ""))
    lines = []
    for mode in modes:
        keep = f"{V}/keep/{name}.{mode}"
        r = subprocess.run(["bash", f"{V}/tools/cc.sh", src, mode, keep, "js-modules=esm", "js-chunk=e0:ent_0"])
        if r.returncode != 0 or open(keep + "/compile.status").read().strip() != "0":
            lines.append(f"# {name} {mode} COMPILE-FAIL see {keep}/compile.log")
            continue
        outdir = "g3" if heavy else "g2"
        for n, (pick, extra) in enumerate(splits):
            called = sorted(p for p in pick if p.startswith("ent_"))
            exp = "-"
            calls = "-"
            if called:
                exp = f"{V}/native/{name}/expect.{n}"
                open(exp, "w").write("".join(sections[int(c[4:])] for c in called))
                calls = ",".join(f"{chunk(c)}:{c}" for c in called)
            args = "js-modules=esm " + " ".join(f"js-chunk={chunk(p)}:{p}" for p in pick) + (" " + extra if extra else "")
            lines.append(f"{name}.{mode}.{n}|{keep}|{V}/out/{outdir}/{name}.{mode}.{n}|{exp}|{calls}|{args}")
    return lines


with ThreadPoolExecutor(workers) as pool, open(jobs_path, "w") as out:
    for lines in pool.map(one, range(first, last + 1)):
        for l in lines:
            out.write(l + "\n")
print("jobs:", sum(1 for l in open(jobs_path) if not l.startswith("#")), "problems:", sum(1 for l in open(jobs_path) if l.startswith("#")))
