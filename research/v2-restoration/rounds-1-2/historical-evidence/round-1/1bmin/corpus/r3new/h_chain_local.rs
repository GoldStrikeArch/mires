//! (h) zero-offset chains four levels deep on the stack: every level shares one address.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use core::hint::black_box;
use core::mem;
use core::ptr;
use prelude::*;

#[repr(C)]
#[derive(Clone, Copy)]
struct D {
    x: u32,
    y: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct C {
    d: D,
    z: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct B {
    c: C,
    w: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct A {
    b: B,
    v: u32,
}
#[repr(C)]
struct Wrap {
    n: u32,
    a: A,
}
#[repr(C)]
struct T4(A, D, u32);

#[inline(never)]
fn mk(s: u32) -> A {
    black_box(A { b: B { c: C { d: D { x: s, y: s + 1 }, z: s + 2 }, w: s + 3 }, v: s + 4 })
}
fn ad<T>(r: &T) -> usize {
    r as *const T as usize
}
fn sum(a: &A) -> u32 {
    a.b.c.d.x + a.b.c.d.y * 10 + a.b.c.z * 100 + a.b.w * 1000 + a.v * 10000
}
#[inline(never)]
fn erased(a: &A) -> *const () {
    a as *const A as *const ()
}
#[inline(never)]
fn inner_d(a: &A) -> *const D {
    &a.b.c.d
}

#[inline(never)]
fn t_local() {
    print_str("local");
    let mut a = mk(1);
    print_bool(ptr::addr_eq(&a, &a.b));
    print_bool(ptr::addr_eq(&a.b, &a.b.c));
    print_bool(ptr::addr_eq(&a.b.c, &a.b.c.d));
    print_bool(ptr::addr_eq(&a, &a.b.c.d));
    print_bool(ptr::addr_eq(&a, &a.b.c.d.x));
    print_bool(!ptr::addr_eq(&a, &a.b.c.d.y));
    print_bool(ad(&a) == ad(&a.b) && ad(&a.b) == ad(&a.b.c) && ad(&a.b.c) == ad(&a.b.c.d));
    print_bool(ad(&a.b.c.d.y) - ad(&a) == mem::offset_of!(D, y));
    print_bool(ad(&a.b.c.z) - ad(&a) == mem::offset_of!(C, z));
    print_bool(ad(&a.b.w) - ad(&a) == mem::offset_of!(B, w));
    print_bool(ad(&a.v) - ad(&a) == mem::offset_of!(A, v));
    print_bool(ad(&a.b.c.z) < ad(&a.b.w) && ad(&a.b.w) < ad(&a.v));
    print_bool(ptr::addr_eq(erased(&a), inner_d(&a)));
    print_bool(ptr::eq(&a.b as *const B as *const C, &a.b.c));
    print_bool(ptr::eq(&raw const a.b.c.d, inner_d(&a)));
    print_bool(&raw const a as usize == &raw const a.b.c.d.x as usize);
    let top = ad(&a);
    let raw_d = &raw const a.b.c.d;
    // stores at every level keep the chain
    a.b.c.d = D { x: 9, y: 8 };
    print_bool(top == ad(&a.b.c.d) && ptr::addr_eq(&a, raw_d));
    a.b.c = mk(2).b.c;
    print_bool(top == ad(&a.b.c.d) && ptr::addr_eq(&a, raw_d));
    a.b = mk(3).b;
    print_bool(top == ad(&a.b.c.d) && ptr::addr_eq(&a, raw_d));
    a = mk(4);
    print_bool(top == ad(&a.b.c.d) && ptr::addr_eq(&a, raw_d));
    print_bool(top == ad(&a) && top == ad(&a.b) && top == ad(&a.b.c));
    print_u32(unsafe { (*raw_d).x + (*raw_d).y });
    print_u32(sum(&a));
    let ra = &mut a;
    *ra = mk(5);
    print_bool(top == ad(&ra.b.c.d.x));
    let rb = &mut ra.b;
    *rb = mk(6).b;
    print_bool(top == ad(rb) && top == ad(&rb.c.d));
    print_u32(sum(&a));
    let other = mk(7);
    print_bool(!ptr::addr_eq(&a, &other) && !ptr::addr_eq(&a.b.c.d, &other.b.c.d));
    print_bool(ptr::addr_eq(&other, &other.b.c.d));
}

#[inline(never)]
fn t_wrapped() {
    print_str("wrapped");
    let mut w = black_box(Wrap { n: 1, a: mk(1) });
    let at = ad(&w.a);
    print_bool(ptr::addr_eq(&w.a, &w.a.b.c.d));
    print_bool(ptr::addr_eq(&w.a.b, &w.a.b.c.d.x));
    print_bool(!ptr::addr_eq(&w, &w.a));
    print_bool(ptr::addr_eq(&w, &w.n));
    print_bool(ad(&w.a.b.c.z) - ad(&w.a) == mem::offset_of!(C, z));
    print_bool(ad(&w.a.v) - ad(&w.a.b.c.d) == mem::offset_of!(A, v));
    w.a = mk(2);
    print_bool(at == ad(&w.a.b.c.d) && at == ad(&w.a.b));
    w = black_box(Wrap { n: 2, a: mk(3) });
    print_bool(at == ad(&w.a.b.c.d) && at == ad(&w.a));
    print_u32(sum(&w.a) + w.n);
    let mut t = black_box(T4(mk(1), D { x: 1, y: 2 }, 3));
    print_bool(ptr::addr_eq(&t, &t.0));
    print_bool(ptr::addr_eq(&t, &t.0.b.c.d.x));
    print_bool(ad(&t.1) - ad(&t.0.b.c.d) == mem::offset_of!(T4, 1));
    print_bool(ad(&t.2) - ad(&t) == mem::offset_of!(T4, 2));
    print_bool(ad(&t.1.y) - ad(&t.1) == mem::offset_of!(D, y));
    let t1 = ad(&t.1);
    t = black_box(T4(mk(2), D { x: 3, y: 4 }, 5));
    print_bool(t1 == ad(&t.1) && ptr::addr_eq(&t, &t.0.b.c.d));
    print_u32(sum(&t.0) + t.1.x + t.2);
    let arr = black_box([mk(1), mk(2)]);
    print_bool(ptr::addr_eq(&arr[1], &arr[1].b.c.d));
    print_bool(!ptr::addr_eq(&arr[0], &arr[1].b.c.d));
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    t_local();
    t_wrapped();
}
