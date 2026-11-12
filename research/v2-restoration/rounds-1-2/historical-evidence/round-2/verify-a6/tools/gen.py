#!/usr/bin/env python3
"""gen.py <seed> <out.rs> [opt]: a random graph of statics that refer to each other.

Three struct types. T0 has one mandatory same-type edge (so T0 always holds cycles), T1 only has
slices (so it may be acyclic), T2 has a pair array. Every type has slices to its own type, down to
the next type and up to the previous one, and a slice of references to boxed `&'static T` statics
(HEADs). With `opt` every type also gets an `Option<&'static T>` and a tagged enum payload edge
(needs the niche-constant decoder fix to be meaningful).

Three entry functions chunk_a/b/c each walk a random subset of the statics to depth 4, adding to an
AtomicU32 in every node they pass, print the checksum, then print the counters of some nodes read
by name and some `ptr::eq` facts. `rust_entry` calls the three in order, so the unsplit output is
the expectation of the split one as well.
"""
import random, sys

seed = int(sys.argv[1]); out = sys.argv[2]; opt = len(sys.argv) > 3
rng = random.Random(seed)
K = 3
counts = [rng.randint(5, 9) for _ in range(K)]
nodes = [[f"N{k}_{i}" for i in range(counts[k])] for k in range(K)]
heads = [[f"H{k}_{i}" for i in range(rng.randint(1, 3))] for k in range(K)]
# shuffle static declaration order through names: prefix letters decide the sorted order
def pick(lst, lo, hi):
    n = rng.randint(lo, min(hi, len(lst)))
    return [rng.choice(lst) for _ in range(n)]

src = []
src.append('#![no_std]\n#![no_main]\n#![allow(dead_code, unused, non_upper_case_globals)]\n#[path = "prelude.rs"]\nmod prelude;\nuse prelude::*;\nuse core::sync::atomic::{AtomicU32, Ordering};\n')
for k in range(K):
    f = [f"id: u32", "hits: AtomicU32", f"same: &'static [&'static T{k}]", f"heads: &'static [&'static &'static T{k}]"]
    if k == 0: f.append(f"one: &'static T{k}")
    if k == 2: f.append(f"pair: [&'static T{k}; 2]")
    if k < K - 1: f.append(f"down: &'static [&'static T{k+1}]")
    if k > 0: f.append(f"up: &'static [&'static T{k-1}]")
    if opt:
        f.append(f"o: Option<&'static T{k}>")
        f.append(f"e: E{k}")
        src.append(f"enum E{k} {{ None, One(&'static T{k}), Two(u32, &'static T{k}, &'static T{k}) }}\n")
    src.append(f"struct T{k} {{ {', '.join(f)} }}\n")

decls = []
nid = 0
ids = {}
for k in range(K):
    for h in heads[k]:
        decls.append(f"static {h}: &T{k} = &{rng.choice(nodes[k])};\n")
    for n in nodes[k]:
        nid += 1; ids[n] = nid
        sparse = rng.random() < 0.35      # some nodes have no outgoing slice edges at all
        def sl(lst, hi):
            return "&[" + ", ".join("&" + x for x in ([] if sparse else pick(lst, 0, hi))) + "]"
        f = [f"id: {nid}", "hits: AtomicU32::new(0)", f"same: {sl(nodes[k], 3)}", f"heads: {sl(heads[k], 2)}"]
        if k == 0: f.append(f"one: &{rng.choice(nodes[k])}")
        if k == 2: f.append(f"pair: [&{rng.choice(nodes[k])}, &{n if rng.random() < 0.3 else rng.choice(nodes[k])}]")
        if k < K - 1: f.append(f"down: {sl(nodes[k+1], 2)}")
        if k > 0: f.append("up: " + ("&[]" if rng.random() < 0.7 else sl(nodes[k-1], 1)))
        if opt:
            f.append("o: " + ("None" if rng.random() < 0.4 else f"Some(&{rng.choice(nodes[k])})"))
            r = rng.random()
            f.append("e: " + (f"E{k}::None" if r < 0.3 else f"E{k}::One(&{rng.choice(nodes[k])})" if r < 0.65 else f"E{k}::Two({rng.randint(1, 9)}, &{rng.choice(nodes[k])}, &{rng.choice(nodes[k])})"))
        decls.append(f"static {n}: T{k} = T{k} {{ {', '.join(f)} }};\n")
rng.shuffle(decls)
src += decls

for k in range(K):
    body = [f"fn walk{k}(n: &'static T{k}, depth: u32) -> u32 {{",
            "    n.hits.fetch_add(1, Ordering::Relaxed);",
            "    if depth == 0 { return n.id; }",
            "    let mut t = n.id.wrapping_mul(31);",
            f"    for s in n.same.iter() {{ t = t.wrapping_mul(7).wrapping_add(walk{k}(s, depth - 1)); }}",
            f"    for h in n.heads.iter() {{ t = t.wrapping_mul(11).wrapping_add(walk{k}(**h, depth - 1)); }}"]
    if k == 0: body.append(f"    t = t.wrapping_mul(13).wrapping_add(walk{k}(n.one, depth - 1));")
    if k == 2: body.append(f"    for p in n.pair.iter() {{ t = t.wrapping_mul(17).wrapping_add(walk{k}(p, depth - 1)); }}")
    if k < K - 1: body.append(f"    for d in n.down.iter() {{ t = t.wrapping_mul(19).wrapping_add(walk{k+1}(d, depth - 1)); }}")
    if k > 0: body.append(f"    for u in n.up.iter() {{ t = t.wrapping_mul(23).wrapping_add(walk{k-1}(u, depth - 1)); }}")
    if opt:
        body.append(f"    if let Some(o) = n.o {{ t = t.wrapping_mul(29).wrapping_add(walk{k}(o, depth - 1)); }} else {{ t ^= 0x55; }}")
        body.append(f"    match &n.e {{ E{k}::None => t ^= 0xAA, E{k}::One(a) => t = t.wrapping_mul(37).wrapping_add(walk{k}(a, depth - 1)), E{k}::Two(x, a, b) => t = t.wrapping_mul(41).wrapping_add(*x).wrapping_add(walk{k}(a, depth - 1)).wrapping_sub(walk{k}(b, depth - 1)) }}")
    body += ["    t", "}"]
    src.append("\n".join(body) + "\n")

allnodes = [(k, n) for k in range(K) for n in nodes[k]]
for c in "abc":
    lines = [f"#[unsafe(no_mangle)]\npub fn chunk_{c}() {{", f'    print_str("{c}");']
    roots = rng.sample(allnodes, rng.randint(3, 7))
    for k, n in roots:
        lines.append(f"    print_u32(walk{k}(&{n}, 4));")
    for k, n in rng.sample(allnodes, 5):
        lines.append(f"    print_u32({n}.hits.load(Ordering::Relaxed));")
    for k in range(K):
        n = rng.choice(nodes[k]); m = rng.choice(nodes[k]); h = rng.choice(heads[k])
        lines.append(f"    print_bool({n}.same.len() > 0 && core::ptr::eq({n}.same[0], &{m}));")
        if k == 0: lines.append(f"    print_bool(core::ptr::eq({n}.one, &{m}) || core::ptr::eq({n}.one.one, &{n}));")
        if k == 2: lines.append(f"    print_bool(core::ptr::eq({n}.pair[1], &{n}) || core::ptr::eq({n}.pair[0], &{m}));")
    lines.append("}")
    src.append("\n".join(lines) + "\n")
src.append("#[unsafe(no_mangle)]\npub fn rust_entry() { chunk_a(); chunk_b(); chunk_c(); }\n")
open(out, "w").write("".join(src))
