//! (h) the same chain through a `Box` and a `Vec` element. Heap elements are a documented
//! limit for field offsets / zero-offset sharing: identity lines may deviate, values may not.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
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

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_str("box");
    let mut bx = Box::new(mk(1));
    // 1-4 IDENTITY (zero-offset sharing in a heap element: documented limit)
    print_bool(ptr::addr_eq(&*bx, &bx.b));
    print_bool(ptr::addr_eq(&*bx, &bx.b.c.d));
    print_bool(ptr::addr_eq(&bx.b, &bx.b.c.d.x));
    print_bool(ptr::addr_eq(&bx.b.c, &bx.b.c.d));
    // 5-6 identity of the element and of each field across stores (documented to hold)
    let (e, f) = (ad(&*bx), ad(&bx.b.c.d));
    *bx = mk(2);
    print_bool(e == ad(&*bx));
    print_bool(f == ad(&bx.b.c.d));
    bx.b = mk(3).b;
    print_bool(f == ad(&bx.b.c.d));
    // values
    print_u32(sum(&bx));
    let raw_d = &raw const bx.b.c.d;
    bx.b.c = mk(4).b.c;
    print_u32(unsafe { (*raw_d).x * 10 + (*raw_d).y });
    print_u32(sum(&bx));
    print_str("vec");
    let mut v: Vec<A> = vec![mk(1), mk(2)];
    // IDENTITY (documented limit)
    print_bool(ptr::addr_eq(&v[1], &v[1].b));
    print_bool(ptr::addr_eq(&v[1], &v[1].b.c.d.x));
    print_bool(ptr::addr_eq(&v[1].b.c, &v[1].b.c.d));
    // holds
    let (e, f) = (ad(&v[1]), ad(&v[1].b.c.d));
    v[1] = mk(5);
    print_bool(e == ad(&v[1]));
    print_bool(f == ad(&v[1].b.c.d));
    print_bool(!ptr::addr_eq(&v[0], &v[1]));
    print_bool(!ptr::addr_eq(&v[0].b.c.d, &v[1].b.c.d));
    // values
    print_u32(sum(&v[0]));
    print_u32(sum(&v[1]));
    let raw_d = &raw const v[1].b.c.d;
    v[1].b = mk(6).b;
    v.swap(0, 1);
    print_u32(unsafe { (*raw_d).x * 10 + (*raw_d).y });
    print_u32(sum(&v[0]));
    print_u32(sum(&v[1]));
}
