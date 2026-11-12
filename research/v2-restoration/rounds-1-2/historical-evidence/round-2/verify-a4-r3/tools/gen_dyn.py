#!/usr/bin/env python3
"""gen_dyn.py <outdir>: one program per (trait-object constant form, observation), to establish what
main did with each construct that is now a compile-time refusal."""
import sys
from pathlib import Path

out = Path(sys.argv[1])
out.mkdir(parents=True, exist_ok=True)

HEAD = """//! {doc}
#![no_std]
#![no_main]
#![allow(dead_code)]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

trait Tr: Sync {{ fn id(&self) -> u32; }}
struct N {{ id: u32 }}
impl Tr for N {{ fn id(&self) -> u32 {{ self.id }} }}
impl Tr for u32 {{ fn id(&self) -> u32 {{ *self }} }}
struct Z;
impl Tr for Z {{ fn id(&self) -> u32 {{ 77 }} }}
static B: N = N {{ id: 2 }};
static P: u32 = 5;
static ZS: Z = Z;
struct Plugin {{ name: &'static str, imp: &'static dyn Tr, prio: u32 }}

{items}

#[unsafe(no_mangle)]
pub fn rust_entry() {{
{body}
}}
"""

# form name -> (items, {observation name -> body})
FORMS = {
    # plain &dyn into a static
    "plain_static": ("static D: &dyn Tr = &B;", {
        "call": "    print_u32(black_box(D).id());",
        "thin_eq": "    print_bool(core::ptr::eq(D as *const dyn Tr as *const N, &B));",
        "size": "    print_usize(core::mem::size_of_val(D));",
        "unused": "    print_u32(B.id);",
    }),
    "plain_static_prim": ("static D: &dyn Tr = &P;", {
        "call": "    print_u32(black_box(D).id());",
        "unused": "    print_u32(P);",
    }),
    "plain_static_zst": ("static D: &dyn Tr = &ZS;", {
        "call": "    print_u32(black_box(D).id());",
    }),
    # Option<&dyn> into a static
    "opt_static": ("static D: Option<&dyn Tr> = Some(&B);", {
        "call": "    print_u32(black_box(&D).map_or(0, |d| d.id()));",
        "is_some": "    print_bool(black_box(&D).is_some());",
        "unused": "    print_u32(B.id);",
    }),
    # promoted targets
    "plain_promoted": ("static D: &dyn Tr = &N { id: 9 };", {
        "call": "    print_u32(black_box(D).id());",
        "unused": "    print_u32(B.id);",
    }),
    "plain_promoted_prim": ("static D: &dyn Tr = &7u32;", {
        "call": "    print_u32(black_box(D).id());",
    }),
    "opt_promoted": ("static D: Option<&dyn Tr> = Some(&N { id: 9 });", {
        "call": "    print_u32(black_box(&D).map_or(0, |d| d.id()));",
        "is_some": "    print_bool(black_box(&D).is_some());",
    }),
    "const_plain_static": ("const D: &dyn Tr = &B;", {
        "call": "    print_u32(black_box(D).id());",
    }),
    "const_plain_promoted": ("const D: &dyn Tr = &N { id: 9 };", {
        "call": "    print_u32(black_box(D).id());",
    }),
    # a struct field
    "field_static": ("static PL: Plugin = Plugin { name: \"plug\", imp: &B, prio: 3 };", {
        "call": "    print_u32(black_box(&PL).imp.id());",
        "other_fields": "    print_str(PL.name); print_u32(black_box(&PL).prio);",
    }),
    "field_promoted": ("static PL: Plugin = Plugin { name: \"plug\", imp: &N { id: 9 }, prio: 3 };", {
        "call": "    print_u32(black_box(&PL).imp.id());",
        "other_fields": "    print_str(PL.name); print_u32(black_box(&PL).prio);",
    }),
    # tables
    "table_static": ("static T: [&dyn Tr; 2] = [&B, &P];", {
        "call": "    for d in black_box(&T) { print_u32(d.id()); }",
        "len": "    print_usize(black_box(&T).len());",
    }),
    "table_promoted": ("static T: &[&dyn Tr] = &[&N { id: 9 }, &7u32, &Z];", {
        "call": "    for d in black_box(T) { print_u32(d.id()); }",
        "len": "    print_usize(black_box(T).len());",
    }),
    # closures as dyn Fn
    "dyn_fn_promoted": ("static F: &(dyn Fn(u32) -> u32 + Sync) = &|x| x + 1;", {
        "call": "    print_u32(black_box(F)(1));",
    }),
    # None controls
    "opt_none": ("static D: Option<&dyn Tr> = None;", {
        "call": "    print_u32(black_box(&D).map_or(0, |d| d.id()));",
        "is_some": "    print_bool(black_box(&D).is_some());",
    }),
    # run-time control: the same objects built at run time
    "runtime_control": ("", {
        "call": "    let d: &dyn Tr = black_box(&B); print_u32(d.id()); let o: Option<&dyn Tr> = Some(&P); print_u32(o.map_or(0, |d| d.id()));",
    }),
}

n = 0
for form, (items, observations) in FORMS.items():
    for obs, body in observations.items():
        name = f"dy_{form}__{obs}"
        (out / f"{name}.rs").write_text(HEAD.format(doc=f"{form}: {obs}", items=items, body=body))
        n += 1
print(n)
