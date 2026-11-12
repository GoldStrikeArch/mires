#!/usr/bin/env python3
"""Generates the s1g extent probes: one program per (formation, shape group).

Each program is a body shared verbatim by a mires shell (no_std + prelude) and a native shell
(std + println shims), so the native run is the oracle for the exact same source.
"""
import os
import sys

OUT = sys.argv[1]

# name, definition, ctor(v), write(place, v), read(place), type, val(n), num(x), copy
SHAPES = []


def shape(name, define, ctor, write, read, ty, val, num, copy=True):
    SHAPES.append(dict(name=name, define=define, ctor=ctor, write=write, read=read, ty=ty,
                       val=val, num=num, copy=copy))


def derive(copy):
    return "#[derive(Clone, Copy)]\n" if copy else ""


def field_shape(name, fields, wfield, wty, ctor_fmt, val, num, copy=True, generic=""):
    shape(name,
          derive(copy) + f"struct {name}{generic} {{ {fields} }}\n",
          lambda v, f=ctor_fmt: f.format(v=v),
          lambda p, v, w=wfield: f"({p}).{w} = {v}",
          lambda p, w=wfield: f"({p}).{w}",
          wty, val, num, copy)


ints = lambda t: (lambda n: f"(({n}) as i64 as {t})")
asnum = lambda x: f"(({x}) as i64)"

field_shape("One", "f: u8", "f", "u8", "One {{ f: {v} }}", ints("u8"), asnum)
field_shape("Two", "f: u8, g: u8", "f", "u8", "Two {{ f: {v}, g: 9 }}", ints("u8"), asnum)
field_shape("Three", "a: u32, b: u32, c: u32", "b", "u32", "Three {{ a: 4, b: {v}, c: 6 }}",
            ints("u32"), asnum)
field_shape("Four", "a: u16, b: u16, c: u16, d: u16", "c", "u16",
            "Four {{ a: 4, b: 5, c: {v}, d: 6 }}", ints("u16"), asnum)
shape("Nested",
      "#[derive(Clone, Copy)]\nstruct Inner { f: u8 }\n#[derive(Clone, Copy)]\nstruct Nested { inner: Inner, x: u8 }\n",
      lambda v: f"Nested {{ inner: Inner {{ f: {v} }}, x: 3 }}",
      lambda p, v: f"({p}).inner.f = {v}",
      lambda p: f"({p}).inner.f",
      "u8", ints("u8"), asnum)
shape("Tup", "#[derive(Clone, Copy)]\nstruct Tup(u16, u16);\n",
      lambda v: f"Tup(5, {v})", lambda p, v: f"({p}).1 = {v}", lambda p: f"({p}).1",
      "u16", ints("u16"), asnum)
for t in ["u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize"]:
    n = "N" + t.upper()
    shape(n, f"#[derive(Clone, Copy)]\nstruct {n}({t});\n",
          lambda v, n=n: f"{n}({v})", lambda p, v: f"({p}).0 = {v}", lambda p: f"({p}).0",
          t, ints(t), asnum)
shape("Flag", "#[derive(Clone, Copy)]\nstruct Flag { on: bool }\n",
      lambda v: f"Flag {{ on: {v} }}", lambda p, v: f"({p}).on = {v}", lambda p: f"({p}).on",
      "bool", lambda n: f"(({n}) % 2 == 1)", asnum)
shape("Real", "#[derive(Clone, Copy)]\nstruct Real { x: f64 }\n",
      lambda v: f"Real {{ x: {v} }}", lambda p, v: f"({p}).x = {v}", lambda p: f"({p}).x",
      "f64", lambda n: f"(({n}) as f64 + 0.5)", lambda x: f"((({x}) * 2.0) as i64)")
shape("Letter", "#[derive(Clone, Copy)]\nstruct Letter { c: char }\n",
      lambda v: f"Letter {{ c: {v} }}", lambda p, v: f"({p}).c = {v}", lambda p: f"({p}).c",
      "char", lambda n: f"char::from_u32(({n}) as u32 + 64).unwrap()", lambda x: f"(({x}) as u32 as i64)")
shape("RefN", "#[derive(Clone, Copy)]\nstruct RefN<'a> { r: &'a u32, n: u32 }\n",
      lambda v: f"RefN {{ r: &R2, n: {v} }}", lambda p, v: f"({p}).n = {v}",
      lambda p: f"(({p}).n + *({p}).r)", "u32", ints("u32"), asnum)
shape("RefW", "#[derive(Clone, Copy)]\nstruct RefW<'a> { r: &'a u32 }\n",
      lambda v: f"RefW {{ r: {v} }}", lambda p, v: f"({p}).r = {v}", lambda p: f"({p}).r",
      "&'static u32", lambda n: f"refv(({n}) as u32)", lambda x: f"(*({x}) as i64)")
shape("Wrap", "#[derive(Clone, Copy)]\nstruct Wrap<T>(T);\n",
      lambda v: f"Wrap::<u32>({v})", lambda p, v: f"({p}).0 = {v}", lambda p: f"({p}).0",
      "u32", ints("u32"), asnum)
shape("Gen", "#[derive(Clone, Copy)]\nstruct Gen<T> { v: T, k: u8 }\n",
      lambda v: f"Gen::<u16> {{ v: {v}, k: 3 }}", lambda p, v: f"({p}).v = {v}",
      lambda p: f"(({p}).v + ({p}).k as u16)", "u16", ints("u16"), asnum)
shape("EnumS", "#[derive(Clone, Copy)]\nenum EnumS { V { f: u8 } }\n",
      lambda v: f"EnumS::V {{ f: {v} }}",
      lambda p, v: f"match &mut ({p}) {{ EnumS::V {{ f }} => *f = {v} }}",
      lambda p: f"(match &({p}) {{ EnumS::V {{ f }} => *f }})", "u8", ints("u8"), asnum)
shape("EnumT", "#[derive(Clone, Copy)]\nenum EnumT { V(u32) }\n",
      lambda v: f"EnumT::V({v})",
      lambda p, v: f"match &mut ({p}) {{ EnumT::V(f) => *f = {v} }}",
      lambda p: f"(match &({p}) {{ EnumT::V(f) => *f }})", "u32", ints("u32"), asnum)
field_shape("NcOne", "f: u8", "f", "u8", "NcOne {{ f: {v} }}", ints("u8"), asnum, copy=False)
field_shape("NcTwo", "f: u8, g: u8", "f", "u8", "NcTwo {{ f: {v}, g: 9 }}", ints("u8"), asnum,
            copy=False)
shape("NcNest", "struct NcIn { f: u8 }\nstruct NcNest { inner: NcIn, x: u8 }\n",
      lambda v: f"NcNest {{ inner: NcIn {{ f: {v} }}, x: 3 }}",
      lambda p, v: f"({p}).inner.f = {v}", lambda p: f"({p}).inner.f", "u8", ints("u8"),
      asnum, copy=False)

GROUPS = {
    "g01": ["One", "Two", "Three"],
    "g02": ["Four", "Nested", "Tup"],
    "g03": ["RefN", "RefW", "Wrap"],
    "g04": ["Gen", "EnumS", "EnumT"],
    "g05": ["NU8", "NU16", "NU32"],
    "g06": ["NU64", "NU128", "NUSIZE"],
    "g07": ["NI8", "NI16", "NI32"],
    "g08": ["NI64", "NI128", "NISIZE"],
    "g09": ["Flag", "Real", "Letter"],
    "g10": ["NcOne", "NcTwo", "NcNest"],
}
BY = {s["name"]: s for s in SHAPES}


def p(s, place):
    return f"print_i64({s['num'](s['read'](place))});"


def bb(s, n):
    return f"black_box({s['val'](n)})"


def init(s, multi):
    lines = [f"    let mut a = {s['ctor'](bb(s, 1))};"]
    if multi:
        lines.append(f"    if black_box(true) {{ a = {s['ctor'](bb(s, 7))}; }}")
    return "\n".join(lines)


TYNAME = {"Wrap": "Wrap<u32>", "Gen": "Gen<u16>", "RefN": "RefN<'z>", "RefW": "RefW<'z>"}


def helpers(form, s, k):
    n, T = s["name"].lower() + "_" + k, s["ty"]
    X = TYNAME.get(s["name"], s["name"])
    lt = "'z, " if "'z" in X else ""
    g = f"<{lt[:-2]}>" if lt else ""
    implx = X.replace("'z", "'_")
    if form in ("helper", "swap"):
        return "".join(f"fn set_{n}{i}{g}(s: &mut {X}, v: {T}) {{ {s['write']('*s', 'v')}; }}\n" for i in (1, 2))
    if form in ("loop", "match"):
        return f"fn set_{n}{g}(s: &mut {X}, v: {T}) {{ {s['write']('*s', 'v')}; }}\n"
    if form == "method":
        return f"impl {implx} {{ fn set_{k}1(&mut self, v: {T}) {{ {s['write']('*self', 'v')}; }} fn set_{k}2(&mut self, v: {T}) {{ {s['write']('*self', 'v')}; }} }}\n"
    if form == "option":
        return (f"fn set_opt_{n}{g}(o: Option<&mut {X}>, v: {T}) {{ if let Some(s) = o {{ {s['write']('*s', 'v')}; }} }}\n")
    if form == "returned":
        return (f"fn pick_{n}<'q, {lt}>(s: &'q mut {X}) -> &'q mut {X} {{ s }}\n"
                f"fn choose_{n}<'a, {lt}>(c: bool, x: &'a mut {X}, y: &'a mut {X}) -> &'a mut {X} {{ if c {{ x }} else {{ y }} }}\n")
    return ""


def body(form, s, multi, k):
    X, n = s["name"], s["name"].lower() + "_" + k
    w = s["write"]
    out = [init(s, multi), "    " + p(s, "a")]
    if form == "helper":
        out += [f"    set_{n}1(&mut a, {bb(s, 254)});", "    " + p(s, "a"),
                "    { let r = &a; " + p(s, "*r") + " }",
                f"    set_{n}2(&mut a, {bb(s, 77)});", "    " + p(s, "a")]
    elif form == "method":
        out += [f"    a.set_{k}1({bb(s, 254)});", "    " + p(s, "a"),
                "    { let r = &a; " + p(s, "*r") + " }",
                f"    a.set_{k}2({bb(s, 77)});", "    " + p(s, "a")]
    elif form == "direct":
        out += ["    { let p = &mut a; " + w("*p", bb(s, 254)) + "; }", "    " + p(s, "a"),
                "    { let p = &mut a; " + w("*p", bb(s, 77)) + "; " + p(s, "*p") + " }",
                "    " + p(s, "a")]
    elif form == "reborrow":
        out += ["    { let p = &mut a; let q = &mut *p; " + w("*q", bb(s, 254)) + "; }",
                "    " + p(s, "a"),
                "    { let p = &mut a; { let q = &mut *p; " + w("*q", bb(s, 77)) + "; } "
                + w("*p", bb(s, 3)) + "; }",
                "    " + p(s, "a")]
    elif form == "refmut":
        out += ["    match a { ref mut s => { " + w("*s", bb(s, 254)) + "; } }",
                "    " + p(s, "a"),
                "    { let ref mut s = a; " + w("*s", bb(s, 77)) + "; }",
                "    " + p(s, "a")]
    elif form == "closure":
        out += ["    { let mut c = |v| " + w("a", "v") + "; c(" + bb(s, 254) + "); }",
                "    " + p(s, "a"),
                "    { let p = &mut a; let mut c = move |v| " + w("*p", "v") + "; c("
                + bb(s, 77) + "); }",
                "    " + p(s, "a")]
    elif form == "option":
        out += ["    { let o = Some(&mut a); if let Some(s) = o { " + w("*s", bb(s, 254)) + "; } }",
                "    " + p(s, "a"),
                f"    set_opt_{n}(Some(&mut a), {bb(s, 77)});",
                "    " + p(s, "a")]
    elif form == "returned":
        b = [f"    let mut b = {s['ctor'](bb(s, 2))};"]
        out += ["    " + w(f"*pick_{n}(&mut a)", bb(s, 254)) + ";", "    " + p(s, "a")] + b + [
            "    " + w(f"*choose_{n}(black_box(true), &mut a, &mut b)", bb(s, 77)) + ";",
            "    " + p(s, "a"), "    " + p(s, "b")]
    elif form == "swap":
        out += [f"    let mut b = {s['ctor'](bb(s, 2))};",
                "    core::mem::swap(&mut a, &mut b);",
                f"    set_{n}1(&mut a, {bb(s, 254)});",
                "    " + p(s, "a"), "    " + p(s, "b"),
                f"    let old = core::mem::replace(&mut a, {s['ctor'](bb(s, 5))});",
                f"    set_{n}2(&mut a, {bb(s, 77)});",
                "    " + p(s, "old"), "    " + p(s, "a")]
    elif form == "loop":
        out += ["    let mut i = 0u32;",
                "    while i < black_box(3u32) {",
                f"        if black_box(i % 2 == 0) {{ a = {s['ctor'](s['val']('(i + 1)'))}; }}",
                f"        set_{n}(&mut a, {bb(s, '(i + 3)')});",
                "        " + p(s, "a"),
                "        i += 1;",
                "    }",
                "    " + p(s, "a")]
    elif form == "match":
        out += ["    match black_box(1u8) {",
                f"        0 => a = {s['ctor'](bb(s, 2))},",
                f"        1 => a = {s['ctor'](bb(s, 3))},",
                "        _ => {}",
                "    }",
                f"    set_{n}(&mut a, {bb(s, 254)});",
                "    " + p(s, "a")]
    return "\n".join(out)


FORMS = ["helper", "method", "direct", "reborrow", "refmut", "closure", "option", "returned",
         "swap", "loop", "match"]

PRE_STATICS = """
static R1: u32 = 1; static R2: u32 = 2; static R3: u32 = 3; static R4: u32 = 4; static R5: u32 = 5;
static R7: u32 = 7; static R77: u32 = 77; static R254: u32 = 254; static R0: u32 = 1000;
fn refv(n: u32) -> &'static u32 {
    match n { 1 => &R1, 2 => &R2, 3 => &R3, 4 => &R4, 5 => &R5, 7 => &R7, 77 => &R77, 254 => &R254, _ => &R0 }
}
"""

MIRES_HEAD = """#![no_std]
#![no_main]
#![allow(warnings)]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;
"""
NATIVE_HEAD = """#![allow(warnings)]
fn print_i32(x: i32) { println!("{x}"); }
fn print_i64(x: i64) { println!("{x}"); }
fn print_bool(x: bool) { println!("{x}"); }
fn main() { rust_entry(); }
"""

os.makedirs(OUT, exist_ok=True)
for form in FORMS:
    for g, names in GROUPS.items():
        shapes = [BY[x] for x in names]
        if form == "swap" or form == "match" or form == "loop" or form == "returned":
            pass
        parts = ["use core::hint::black_box;", PRE_STATICS]
        calls = []
        for s in shapes:
            parts.append(s["define"])
            for multi in (True, False):
                k = "m" if multi else "s"
                parts.append(helpers(form, s, k))
                fname = f"t_{s['name'].lower()}_{'multi' if multi else 'single'}"
                parts.append(f"fn {fname}() {{\n{body(form, s, multi, k)}\n}}\n")
                calls.append(f"    print_i32(-1); {fname}();")
        parts.append("#[unsafe(no_mangle)]\npub fn rust_entry() {\n" + "\n".join(calls) + "\n}\n")
        text = "\n".join(parts)
        name = f"{form}_{g}"
        with open(os.path.join(OUT, name + "_m.rs"), "w") as f:
            f.write(MIRES_HEAD + text)
        with open(os.path.join(OUT, name + "_n.rs"), "w") as f:
            f.write(NATIVE_HEAD + text)
print("generated", len(FORMS) * len(GROUPS), "programs")
