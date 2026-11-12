//! Micro-benchmark 2: whole-aggregate stores of shapes the first benchmark does not cover:
//! more than eight leaves (the runtime fallback), a scalar array inside the stored struct, a
//! borrowed local assigned again in a loop, and an enum store.
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
struct Big {
    a: u32, b: u32, c: u32, d: u32, e: u32, f: u32, g: u32, h: u32, i: u32, j: u32, k: u32, l: u32,
}
#[derive(Clone, Copy)]
struct Small {
    x: u32,
    y: u32,
    z: u32,
}
#[derive(Clone, Copy)]
struct WithArr {
    p: Small,
    arr: [u32; 4],
    n: u32,
}
#[derive(Clone, Copy)]
enum E {
    A { x: u32, y: u32 },
    B { s: Small, k: u32 },
}
struct Outer {
    n: u32,
    big: Big,
    wa: WithArr,
    e: E,
}
#[inline(always)]
fn big(i: u32, acc: u32) -> Big {
    Big { a: i, b: acc, c: i ^ 1, d: 3, e: 4, f: i & 7, g: 6, h: 7, i: 8, j: 9, k: acc & 3, l: 11 }
}
#[inline(never)]
fn outer() -> Outer {
    black_box(Outer {
        n: 1,
        big: big(0, 0),
        wa: WithArr { p: Small { x: 0, y: 0, z: 0 }, arr: [0; 4], n: 0 },
        e: E::A { x: 0, y: 0 },
    })
}

#[unsafe(no_mangle)]
pub fn b2_big_field(n: u32) -> u32 {
    let mut o = outer();
    let mut acc = 0u32;
    for i in 0..n {
        o.big = big(i, acc);
        acc = acc.wrapping_add(o.big.a ^ o.big.k);
    }
    acc.wrapping_add(o.n)
}

#[unsafe(no_mangle)]
pub fn b2_big_through(n: u32) -> u32 {
    let mut arr = black_box([big(0, 0); 4]);
    let mut acc = 0u32;
    for i in 0..n {
        let slot = &mut arr[(i & 3) as usize];
        *slot = big(i, acc);
        acc = acc.wrapping_add(slot.a ^ slot.f);
    }
    acc
}

#[unsafe(no_mangle)]
pub fn b2_big_vec(n: u32) -> u32 {
    let mut v: Vec<Big> = Vec::new();
    for i in 0..16u32 {
        v.push(big(i, i));
    }
    let mut v = black_box(v);
    let mut acc = 0u32;
    for i in 0..n {
        let k = (i & 15) as usize;
        v[k] = big(i, acc);
        acc = acc.wrapping_add(v[(i.wrapping_add(5) & 15) as usize].a);
    }
    acc
}

#[inline(never)]
fn step_small(s: &Small, i: u32) -> Small {
    Small { x: s.y.wrapping_add(i), y: s.z, z: s.x }
}
#[inline(never)]
fn step_big(s: &Big, i: u32) -> Big {
    Big { a: s.l.wrapping_add(i), b: s.a, c: s.b, d: s.c, e: s.d, f: s.e, g: s.f, h: s.g, i: s.h, j: s.i, k: s.j, l: s.k }
}

#[unsafe(no_mangle)]
pub fn b2_local_small(n: u32) -> u32 {
    let mut s = black_box(Small { x: 1, y: 2, z: 3 });
    for i in 0..n {
        s = step_small(&s, i);
    }
    s.x ^ s.y ^ s.z
}

#[unsafe(no_mangle)]
pub fn b2_local_big(n: u32) -> u32 {
    let mut s = black_box(big(1, 2));
    for i in 0..n {
        s = step_big(&s, i);
    }
    s.a ^ s.f ^ s.l
}

#[unsafe(no_mangle)]
pub fn b2_with_array(n: u32) -> u32 {
    let mut o = outer();
    let mut acc = 0u32;
    for i in 0..n {
        o.wa = WithArr { p: Small { x: i, y: acc, z: 1 }, arr: [i, i ^ 1, acc & 3, 7], n: i & 1 };
        acc = acc.wrapping_add(o.wa.arr[(i & 3) as usize] ^ o.wa.p.x);
    }
    acc
}

#[unsafe(no_mangle)]
pub fn b2_enum(n: u32) -> u32 {
    let mut o = outer();
    let mut acc = 0u32;
    for i in 0..n {
        o.e = if i & 1 == 0 { E::A { x: i, y: acc } } else { E::B { s: Small { x: i, y: 1, z: 2 }, k: acc & 7 } };
        acc = acc.wrapping_add(match &o.e {
            E::A { x, .. } => *x,
            E::B { s, k } => s.x ^ k,
        });
    }
    acc
}

#[unsafe(no_mangle)]
pub fn b2_swap_big(n: u32) -> u32 {
    let mut a = black_box(big(1, 2));
    let mut b = black_box(big(3, 4));
    let mut acc = 0u32;
    for i in 0..n {
        mem::swap(&mut a, &mut b);
        a.a = a.a.wrapping_add(i);
        acc = acc.wrapping_add(b.a);
    }
    acc
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(b2_big_field(1000));
    print_u32(b2_big_through(1000));
    print_u32(b2_big_vec(1000));
    print_u32(b2_local_small(1000));
    print_u32(b2_local_big(1000));
    print_u32(b2_with_array(1000));
    print_u32(b2_enum(1000));
    print_u32(b2_swap_big(1000));
}
