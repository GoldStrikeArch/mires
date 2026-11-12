//! Micro-benchmark: whole-aggregate stores into fields, Vec elements, swaps, and moves.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::vec::Vec;
use core::hint::black_box;
use core::mem;
use prelude::*;

#[derive(Clone, Copy)]
struct P {
    x: u32,
    y: u32,
}
#[derive(Clone, Copy)]
struct S {
    p: P,
    z: u32,
}
struct Inner {
    v: u32,
    w: u32,
}
struct Outer {
    n: u32,
    s: S,
    inner: Inner,
}

#[unsafe(no_mangle)]
pub fn bench_field(n: u32) -> u32 {
    let mut o = black_box(Outer { n: 1, s: S { p: P { x: 0, y: 0 }, z: 0 }, inner: Inner { v: 0, w: 0 } });
    let mut acc = 0u32;
    for i in 0..n {
        o.s = S { p: P { x: i, y: i ^ 1 }, z: acc & 7 };
        acc = acc.wrapping_add(o.s.p.x ^ o.s.z);
    }
    acc.wrapping_add(o.n)
}

#[unsafe(no_mangle)]
pub fn bench_vec(n: u32) -> u32 {
    let mut v: Vec<S> = Vec::new();
    for i in 0..64u32 {
        v.push(S { p: P { x: i, y: i }, z: i });
    }
    let v = black_box(v);
    let mut v = v;
    let mut acc = 0u32;
    for i in 0..n {
        let k = (i & 63) as usize;
        v[k] = S { p: P { x: i, y: acc }, z: k as u32 };
        acc = acc.wrapping_add(v[(i.wrapping_add(17) & 63) as usize].p.x);
    }
    acc
}

#[unsafe(no_mangle)]
pub fn bench_swap(n: u32) -> u32 {
    let mut a = black_box(S { p: P { x: 1, y: 2 }, z: 3 });
    let mut b = black_box(S { p: P { x: 4, y: 5 }, z: 6 });
    let mut acc = 0u32;
    for i in 0..n {
        mem::swap(&mut a, &mut b);
        a.p.x = a.p.x.wrapping_add(i);
        acc = acc.wrapping_add(b.p.x);
    }
    acc
}

#[unsafe(no_mangle)]
pub fn bench_move(n: u32) -> u32 {
    let mut o = black_box(Outer { n: 1, s: S { p: P { x: 0, y: 0 }, z: 0 }, inner: Inner { v: 1, w: 2 } });
    let mut acc = 0u32;
    for i in 0..n {
        let x = o.inner;
        o.inner = Inner { v: x.v.wrapping_add(i), w: x.w };
        acc = acc.wrapping_add(x.v ^ o.inner.w);
    }
    acc
}

#[unsafe(no_mangle)]
pub fn bench_through(n: u32) -> u32 {
    let mut arr = black_box([S { p: P { x: 0, y: 0 }, z: 0 }; 8]);
    let mut acc = 0u32;
    for i in 0..n {
        let slot = &mut arr[(i & 7) as usize];
        *slot = S { p: P { x: i, y: 0 }, z: acc & 3 };
        acc = acc.wrapping_add(slot.p.x);
    }
    acc
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(bench_field(1000));
    print_u32(bench_vec(1000));
    print_u32(bench_swap(1000));
    print_u32(bench_move(1000));
    print_u32(bench_through(1000));
}
