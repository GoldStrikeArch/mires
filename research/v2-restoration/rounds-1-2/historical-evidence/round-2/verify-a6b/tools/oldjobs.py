#!/usr/bin/env python3
"""oldjobs.py <modes csv> <jobs file> [workers]

Job lines (for tools/probe.py) over the chunk programs that existed before A6b, every one split by
FUNCTION entry points only, so every job has to come out SAME-OK with byte-identical files:

  * the earlier verifier's ck_own (3 chunks), ck_five (5), and ck_six (6, ck_five plus one);
  * its 36 generated static graphs gen_1..24, geno_1..12 (3 chunks);
  * the committed fixtures 262, 263, 264, and 265/266 split by their functions.

Request orders: every permutation up to five chunks; for six, all 720 in the first mode and 40
random ones in the others. On the identity order also: the registry, a renamed shared chunk,
island roles (the layout mires-build's `Chunking` generates), all three together, and every
subset that leaves one entry out.

The expectation is the native run (verify-a6/tools/native.sh, -O0 and -O3 agreeing) for the
scratch programs and the committed `.expected` for fixtures. The geno graphs hold `Option<&T>`
statics that main decodes wrong (known, A4), so they are loaded and run without an expectation.
"""
import itertools, os, random, subprocess, sys
from concurrent.futures import ThreadPoolExecutor

V = "/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6b"
A6 = "/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6"
W = "/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-acddce572a795adec"
modes, jobs_path = sys.argv[1].split(","), sys.argv[2]
workers = int(sys.argv[3]) if len(sys.argv) > 3 else 6
core = W + "/examples/core-tests"
abc = ["chunk_a", "chunk_b", "chunk_c"]
programs = [("ck_own", f"{V}/src/ck_own.rs", abc, "native"),
            ("ck_five", f"{V}/src/ck_five.rs", abc + ["chunk_d", "chunk_e"], "native"),
            ("ck_six", f"{V}/src/ck_six.rs", abc + ["chunk_d", "chunk_e", "chunk_f"], "native")]
programs += [(f"gen_{n}", f"{V}/src/gen_{n}.rs", abc, "native") for n in range(1, 25)]
programs += [(f"geno_{n}", f"{V}/src/geno_{n}.rs", abc, "none") for n in range(1, 13)]
programs += [("fx262", f"{core}/262_chunk_owned_statics.rs", abc, f"{core}/262_chunk_owned_statics.expected"),
             ("fx263", f"{core}/263_chunk_three_owners.rs", abc, f"{core}/263_chunk_three_owners.expected"),
             ("fx264", f"{core}/264_chunk_static_cycles.rs", abc[:2], f"{core}/264_chunk_static_cycles.expected"),
             ("fx265", f"{core}/265_chunk_static_entries.rs", abc[:2], f"{core}/265_chunk_static_entries.expected"),
             ("fx266", f"{core}/266_chunk_static_entry_refusals.rs", abc[:2], f"{core}/266_chunk_static_entry_refusals.expected")]

only = os.environ.get("OLD_ONLY")
if only:
    programs = [p for p in programs if p[0].startswith(tuple(only.split(",")))]

def one(program):
    name, src, entries, expect = program
    if expect == "native":
        nat = subprocess.run(["bash", f"{A6}/tools/native.sh", src, f"{V}/native/{name}"], capture_output=True, text=True)
        if nat.returncode != 0:
            return [f"# {name} NATIVE-FAIL " + nat.stdout[:200].replace("\n", " ")]
        expect = nat.stdout.strip()
    elif expect == "none":
        expect = "-"
    chunk = lambda e: e[len("chunk_"):]
    pair = lambda e: f"{chunk(e)}:{e}"
    calls = ",".join(pair(e) for e in entries)
    lines = []
    for at, mode in enumerate(modes):
        keep = f"{V}/keep/{name}.{mode}"
        r = subprocess.run(["bash", f"{V}/tools/cc.sh", src, mode, keep, "js-modules=esm", f"js-chunk={pair(entries[0])}"])
        if r.returncode != 0 or open(keep + "/compile.status").read().strip() != "0":
            lines.append(f"# {name} {mode} COMPILE-FAIL see {keep}/compile.log")
            continue
        orders = list(itertools.permutations(entries))
        if len(entries) == 6 and mode != "def":
            orders = random.Random(len(name) + at).sample(orders, 40)
        n = 0
        def add(order, extra="", called=None):
            nonlocal n
            c = calls if called is None else ",".join(pair(e) for e in called)
            args = "js-modules=esm " + " ".join(f"js-chunk={pair(e)}" for e in order) + (" " + extra if extra else "")
            lines.append(f"{name}.{mode}.{n}|{keep}|{V}/out/old/{name}.{mode}.{n}|{expect}|{c}|{args}")
            n += 1
        for order in orders:
            add(order)
        roles = " ".join(f"js-chunk-role={chunk(e)}:island" for e in entries)
        add(entries, "js-chunk-registry=on")
        add(entries, "js-chunk-shared=common")
        add(entries, roles)
        add(entries, f"js-chunk-registry=on js-chunk-shared=common {roles}")
        add(entries, f"js-chunk-role={chunk(entries[0])}:main")
        if expect == "-" or len(entries) > 2:
            for leave in entries:
                rest = [e for e in entries if e != leave]
                # Without an entry the native expectation no longer applies; load and run only.
                lines.append(f"{name}.{mode}.sub-{chunk(leave)}|{keep}|{V}/out/old/{name}.{mode}.sub-{chunk(leave)}|-|{','.join(pair(e) for e in rest)}|js-modules=esm " + " ".join(f"js-chunk={pair(e)}" for e in rest))
    return lines


with ThreadPoolExecutor(workers) as pool, open(jobs_path, "w") as out:
    for lines in pool.map(one, programs):
        for l in lines:
            out.write(l + "\n")
print("jobs:", sum(1 for l in open(jobs_path) if not l.startswith("#")), "problems:", sum(1 for l in open(jobs_path) if l.startswith("#")))
