#!/usr/bin/env python3
"""gen2.py <seed> <out.rs> <out.meta>: a random program whose statics AND functions can be chunk entries.

Statics `S_i` of one struct type hold: an id, a slice of references to other statics (any index,
so cycles happen), a fn pointer to a helper, a slice of zero or one fn pointer to an entry
FUNCTION, and a long string (a hoisted constant under js-hoist-consts=on). About half the statics
are `#[no_mangle]`, so a split may name them as entry points. `P_i` are boxed statics
(`&'static Node`, a `let P = [S]` binding that names another binding), some exported too.
Helpers read statics, call later helpers, name entry functions behind a run-time-false guard, and
make trait objects at run time (vtables, with a supertrait vtable inside, whose methods read
statics and call helpers). Entry functions `ent_k` walk statics, call helpers, name other entry
functions behind the guard, and print.

(No `Option<&T>`, `Option<fn>` or `&dyn Tr` inside a static: main decodes those wrong, known.)

`rust_entry` prints `== k` before calling `ent_k`, so the native output splits into one section
per entry function; nothing is mutated, so a section does not depend on what ran before it.
The meta file lists the entry candidates: `fn ent_0 ...` and `static S_1 ...`.
"""
import os, random, sys

seed = int(sys.argv[1]); out = sys.argv[2]; meta = sys.argv[3]
rng = random.Random(seed)
NS = rng.randint(3, 8)
NH = rng.randint(1, 3)
heavy = bool(os.environ.get('G2_HEAVY'))  # function entries only: more of them, real calls between them, more dyn and panics
NE = rng.randint(3, 5) if heavy else rng.randint(2, 3)
ND = rng.randint(1, 2)
NP = rng.randint(0, 2)
dense = rng.choice([0.1, 0.25, 0.5])
strings = ["the quick brown fox jumps over the lazy dog, twice over",
           "a second long string literal that the compiler may hoist",
           "three long strings make a pool of hoisted constants ok"]
src = ['#![no_std]\n#![no_main]\n#![allow(dead_code, unused, non_upper_case_globals, private_interfaces)]\n#[path = "prelude.rs"]\nmod prelude;\nuse prelude::*;\nuse core::sync::atomic::{AtomicU32, Ordering};\n']
src.append("static ZERO: AtomicU32 = AtomicU32::new(0);\n")
src.append("trait Sup { fn s(&self) -> u32; }\n")
src.append("trait Tr: Sup + Sync { fn v(&self) -> u32; fn w(&self) -> u32 { self.v().wrapping_mul(3).wrapping_add(1) } }\n")
src.append("struct Node { id: u32, kids: &'static [&'static Node], f: fn() -> u32, g: &'static [fn()], s: &'static str }\n")
exported = [i for i in range(NS) if rng.random() < 0.55] or [0]
pexported = [k for k in range(NP) if rng.random() < 0.6]
for k in range(ND):
    how = rng.choice(["static", "static", "plain"])
    body = {"static": f"self.0.wrapping_add(S_{rng.randrange(NS)}.id)", "helper": f"self.0.wrapping_add(h_{rng.randrange(NH)}())", "plain": "self.0"}[how]
    src.append(f"struct Imp{k}(u32);\nimpl Sup for Imp{k} {{ fn s(&self) -> u32 {{ self.0.wrapping_add(S_{rng.randrange(NS)}.id) }} }}\nimpl Tr for Imp{k} {{ fn v(&self) -> u32 {{ {body} }} }}\nstatic D_{k}: Imp{k} = Imp{k}({k + 5});\n")
decls = []
for i in range(NS):
    kids = [j for j in range(NS) if rng.random() < dense / 2 and (j != i or rng.random() < 0.3)][:3]
    f = f"h_{rng.randrange(NH)}"
    g = f"&[ent_{rng.randrange(NE)}]" if rng.random() < 0.2 else "&[]"
    s = strings[rng.randrange(len(strings))]
    attr = "#[unsafe(no_mangle)]\npub " if i in exported else ""
    decls.append(f"{attr}static S_{i}: Node = Node {{ id: {i + 1}, kids: &[{', '.join('&S_%d' % j for j in kids)}], f: {f}, g: {g}, s: \"{s}\" }};\n")
for k in range(NP):
    attr = "#[unsafe(no_mangle)]\npub " if k in pexported else ""
    decls.append(f"{attr}static P_{k}: &'static Node = &S_{rng.randrange(NS)};\n")
rng.shuffle(decls)
src += decls
src.append("""fn walk(n: &'static Node, depth: u32) -> u32 {
    let mut t = n.id.wrapping_mul(31).wrapping_add((n.f)()).wrapping_add(n.s.len() as u32);
    if n.g.len() > 0 { t = t.wrapping_add(1000); }
    if depth > 0 { for k in n.kids.iter() { t = t.wrapping_mul(7).wrapping_add(walk(k, depth - 1)); } }
    t
}
#[inline(never)]
fn dynv(d: &dyn Tr) -> u32 { let s: &dyn Sup = d; d.v().wrapping_add(d.w()).wrapping_add(s.s()) }
""")
for j in range(NH):
    parts = [f"S_{rng.randrange(NS)}.id"]
    if j + 1 < NH and rng.random() < 0.5:
        parts.append(f"h_{rng.randrange(j + 1, NH)}()")
    if rng.random() < 0.4:
        parts.append(f"(\"{strings[rng.randrange(len(strings))]}\".len() as u32)")
    if rng.random() < (0.75 if heavy else 0.4):
        parts.append(f"dynv(&D_{rng.randrange(ND)})")
    if heavy and rng.random() < 0.6:
        parts.append(f"Some(S_{rng.randrange(NS)}.id).filter(|v| *v < 1000).expect(\"an id below one thousand, as every generated id is\")")
    if NP and rng.random() < 0.4:
        parts.append(f"P_{rng.randrange(NP)}.id")
    guard = f"    if ZERO.load(Ordering::Relaxed) != 0 {{ ent_{rng.randrange(NE)}(); }}\n" if rng.random() < 0.3 else ""
    src.append(f"#[inline(never)]\nfn h_{j}() -> u32 {{\n{guard}    {'.wrapping_add('.join(parts) + ')' * (len(parts) - 1)}\n}}\n")
for k in range(NE):
    lines = [f"#[unsafe(no_mangle)]\npub fn ent_{k}() {{"]
    for _ in range(rng.randint(1, 3)):
        lines.append(f"    print_u32(walk(&S_{rng.randrange(NS)}, 3));")
    if rng.random() < 0.6:
        lines.append(f"    print_u32(h_{rng.randrange(NH)}());")
    if rng.random() < 0.4:
        lines.append(f"    print_u32(dynv(&D_{rng.randrange(ND)}));")
    if NP and rng.random() < 0.5:
        lines.append(f"    print_u32(walk(P_{rng.randrange(NP)}, 2));")
    if heavy and k + 1 < NE and rng.random() < 0.5:
        lines.append(f"    ent_{rng.randrange(k + 1, NE)}();")
    if rng.random() < 0.35:
        other = rng.choice([m for m in range(NE) if m != k])
        lines.append(f"    if ZERO.load(Ordering::Relaxed) != 0 {{ ent_{other}(); }}")
    a, b = rng.randrange(NS), rng.randrange(NS)
    lines.append(f"    print_bool(S_{a}.kids.len() > 0 && core::ptr::eq(S_{a}.kids[0], &S_{b}));")
    if rng.random() < 0.5:
        lines.append(f"    print_str(S_{rng.randrange(NS)}.s);")
    lines.append("}")
    src.append("\n".join(lines) + "\n")
src.append("#[unsafe(no_mangle)]\npub fn rust_entry() { " + " ".join(f'print_str("== {k}"); ent_{k}();' for k in range(NE)) + " }\n")
open(out, "w").write("".join(src))
open(meta, "w").write("".join(f"fn ent_{k}\n" for k in range(NE)) + "".join(f"static S_{i}\n" for i in exported) + "".join(f"static P_{k}\n" for k in pexported))
