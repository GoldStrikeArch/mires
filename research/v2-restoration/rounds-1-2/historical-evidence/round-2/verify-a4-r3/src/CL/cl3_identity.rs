//! Pointer identity of closure fn-pointer constants.
#![no_std]
#![no_main]
#![allow(unpredictable_function_pointer_comparisons)]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

type F = fn(u32) -> u32;
fn named(x: u32) -> u32 { x * 3 }
const C1: F = |x| x + 7;
const C2: F = |x| x * 11 + 1;
const NAMED: F = named;
static S1: F = |x| x + 9;
static S2: F = |x| x * 13 + 2;
static TABLE: [F; 3] = [|x| x + 1, named, |x| x * x + 5];
const CTABLE: [F; 2] = [|x| x + 21, |x| x * 23 + 3];
const OC: Option<F> = Some(|x| x + 31);
struct Op { run: F }
static OPS: [Op; 2] = [Op { run: |x| x + 41 }, Op { run: |x| x * 43 + 4 }];
const COP: Op = Op { run: |x| x + 51 };
const PROM: &F = &((|x| x + 61) as F);

#[inline(never)]
fn same(a: F, b: F) -> bool { black_box(a) == black_box(b) }
#[inline(never)]
fn addr_same(a: F, b: F) -> bool { core::ptr::fn_addr_eq(black_box(a), black_box(b)) }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // the same constant twice
    print_bool(same(C1, C1));
    print_bool(addr_same(C1, C1));
    print_bool(same(NAMED, NAMED));
    print_bool(same(NAMED, named));
    let a = C1; let b = C1;
    print_bool(a == b);
    print_bool(C1 as *const () == C1 as *const ());
    // different constants
    print_bool(same(C1, C2));
    print_bool(same(C1, NAMED));
    // statics
    print_bool(same(S1, S1));
    print_bool(addr_same(S1, S1));
    print_bool(same(S1, S2));
    let p = &S1; let q = &S1;
    print_bool(*p == *q);
    print_bool(core::ptr::eq(p, q));
    // tables
    print_bool(same(TABLE[0], TABLE[0]));
    print_bool(same(TABLE[black_box(2)], TABLE[black_box(2)]));
    print_bool(same(TABLE[0], TABLE[2]));
    print_bool(same(TABLE[1], named));
    print_bool(same(CTABLE[0], CTABLE[0]));
    print_bool(same(CTABLE[0], CTABLE[1]));
    let t = CTABLE;
    print_bool(same(t[1], t[1]));
    // Option and struct
    print_bool(OC == OC);
    print_bool(OC.map_or(false, |f| same(f, f)));
    print_bool(same(OPS[0].run, OPS[0].run));
    print_bool(same(OPS[0].run, OPS[1].run));
    print_bool(same(COP.run, COP.run));
    print_bool(same(*PROM, *PROM));
    // a copy of a constant compares equal to itself
    let c = black_box(C2);
    print_bool(same(c, c));
    // loop: the same constant evaluated on each iteration
    let mut prev: Option<F> = None;
    let mut all = true;
    for _ in 0..3 {
        let cur: F = black_box(C1);
        if let Some(p) = prev { all &= p == cur; }
        prev = Some(cur);
    }
    print_bool(all);
    print_u32(C1(1) + S1(1) + TABLE[2](2));
}
