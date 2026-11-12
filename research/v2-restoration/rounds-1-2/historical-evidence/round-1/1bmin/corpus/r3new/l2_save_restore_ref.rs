//! (l2) save / overwrite / restore THROUGH a pointer: `let saved = *r; *r = x; ...; *r = saved`.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use core::hint::black_box;
use prelude::*;

#[derive(Clone, Copy)]
struct S {
    a: u32,
    b: u32,
}
struct W {
    depth: u32,
    s: S,
}
#[inline(never)]
fn peek(r: &u32) -> u32 {
    *r
}
#[inline(never)]
fn peek_s(r: &S) -> u32 {
    r.a * 10 + r.b
}
#[inline(never)]
fn scalar(r: &mut u32) -> u32 {
    let saved = *r;
    *r = 5;
    let mid = peek(r);
    *r = saved;
    mid
}
#[inline(never)]
fn scalar_raw(r: *mut u32) -> u32 {
    unsafe {
        let saved = *r;
        *r = 6;
        let mid = peek(&*r);
        *r = saved;
        mid
    }
}
#[inline(never)]
fn agg(r: &mut S) -> u32 {
    let saved = *r;
    *r = S { a: 7, b: 7 };
    let mid = peek_s(r);
    *r = saved;
    mid
}
#[inline(never)]
fn field(w: &mut W) -> u32 {
    let saved = w.depth;
    w.depth = 9;
    let mid = peek(&w.depth);
    w.depth = saved;
    let saved_a = w.s.a;
    w.s.a = 8;
    let mid2 = peek_s(&w.s);
    w.s.a = saved_a;
    mid * 1000 + mid2
}
#[inline(never)]
fn recurse(depth: &mut u32, n: u32) -> u32 {
    if n == 0 {
        return *depth;
    }
    let old = *depth;
    *depth += 1;
    let r = recurse(depth, n - 1);
    *depth = old;
    r
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut x = black_box(1u32);
    print_u32(scalar(&mut x) * 10 + x);
    print_u32(scalar_raw(&mut x) * 10 + x);
    let mut s = black_box(S { a: 1, b: 2 });
    print_u32(agg(&mut s) * 100 + s.a * 10 + s.b);
    let mut w = black_box(W { depth: 1, s: S { a: 2, b: 3 } });
    print_u32(field(&mut w));
    print_u32(w.depth * 100 + w.s.a * 10 + w.s.b);
    let mut d = black_box(3u32);
    print_u32(recurse(&mut d, 4) * 10 + d);
}
