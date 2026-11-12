#!/usr/bin/env python3
"""cyclestat.py <jobs file> <results.tsv>: what the accepted (SAME-OK) splits look like.

For every SAME-OK job, reads the chunk files the linker under test wrote, builds the graph of
`import .. from "./x.js"` between them, and counts: splits with a static entry point, splits whose
files import each other in a cycle, and splits where a static-entry chunk sits on such a cycle
(accepted because only functions cross it). Also scans every module level statement of every
accepted, unminified chunk file for anything but an import, an export, a function declaration, a
`let`/`const` binding, an `Object.assign` fill or the registry line, and for a call inside a
binding initializer other than `__rt.box(..)`.
"""
import os, re, sys

jobs = {}
for l in open(sys.argv[1]):
    if l.startswith("#") or not l.strip():
        continue
    ident, keep, out, exp, calls, args = l.rstrip("\n").split("|")
    jobs[ident] = (out, args)
total = static_entry = cyc = cyc_static = 0
odd = {}
calls_in_init = {}
examples = []
for row in open(sys.argv[2]):
    f = row.rstrip("\n").split("\t")
    if f[1] != "SAME-OK":
        continue
    out, args = jobs[f[0]]
    total += 1
    entries = dict(a[len("js-chunk="):].split(":") for a in args.split() if a.startswith("js-chunk="))
    statics = {c for c, e in entries.items() if not (e.startswith("ent_") or e.startswith("chunk_"))}
    if statics:
        static_entry += 1
    d = out + "/fix"
    graph = {}
    for name in os.listdir(d):
        if not name.endswith(".js") or name == "program.js":
            continue
        text = open(f"{d}/{name}").read()
        graph[name[:-3]] = set(m for m in re.findall(r'from ?"\./([^"]+)\.js"', text) if m != "shim")
        if "\n" in text.strip() and not text.startswith("import{") and "let " in text and " = " in text:
            for line in text.split("\n"):
                if not line or line[0] in " }\t/":
                    continue
                if re.match(r"(import |export |function |let |const |Object\.assign\(|\(globalThis\.__rcgjsChunks)", line):
                    if re.match(r"(let |const |Object\.assign\()", line):
                        body = re.sub(r'"(\\.|[^"\\])*"', '""', line)
                        body = re.sub(r"^Object\.assign\(", "", body)
                        for call in re.findall(r"([A-Za-z_$][\w$.]*)\(", body):
                            if call != "__rt.box":
                                calls_in_init[call] = calls_in_init.get(call, 0) + 1
                    continue
                odd[line[:60]] = odd.get(line[:60], 0) + 1
    def reach(a):
        seen, work = set(), [a]
        while work:
            for n in graph.get(work.pop(), ()):
                if n not in seen:
                    seen.add(n); work.append(n)
        return seen
    on_cycle = {n for n in graph if n in reach(n)}
    if on_cycle:
        cyc += 1
        if on_cycle & statics:
            cyc_static += 1
            if len(examples) < 5:
                examples.append(f"{f[0]}: cycle over {sorted(on_cycle)}, static chunks on it {sorted(on_cycle & statics)}")
print(f"accepted {total}; with a static entry {static_entry}; with an import cycle {cyc}; with a static-entry chunk on an import cycle {cyc_static}")
for e in examples:
    print("  e.g.", e)
print("module level statements of another kind:", odd or "none")
print("calls inside binding initializers other than __rt.box:", calls_in_init or "none")
