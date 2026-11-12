//! Verifier fixture (p1_place): aggregate-place relations: a reference to a nested aggregate against the raw place
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::ptr;

#[derive(Clone, Copy)]
struct P {
    x: u32,
    y: u32,
}

struct R<'a> {
    n: u32,
    r: &'a P,
}

struct Guard(u32);

impl Drop for Guard {
    fn drop(&mut self) {
        print_u32(self.0);
    }
}

#[inline(never)]
fn mk(x: u32) -> P {
    black_box(P { x, y: x + 1 })
}

#[inline(never)]
fn address(p: *const P) -> usize {
    p as usize
}

macro_rules! plain_and_guarded {
    ($plain:ident, $guarded:ident, $tag:expr, $body:block) => {
        #[inline(never)]
        fn $plain() $body

        #[inline(never)]
        fn $guarded() {
            let _guard = Guard($tag);
            $body
        }
    };
}

struct H {
    head: u32,
    tail: u32,
}

struct Outer {
    n: u32,
    h: H,
    arr: [P; 2],
}

plain_and_guarded!(place_identity, place_identity_guarded, 401, {
    let mut o = black_box(Outer { n: 17, h: H { head: 11, tail: 12 }, arr: [mk(1), mk(2)] });
    print_bool(&o.h as *const H == &raw const o.h);
    print_bool(ptr::eq(&o.h, &raw const o.h));
    print_bool(&o.arr[1] as *const P == &raw const o.arr[1]);
    print_bool(&o.h.head as *const u32 as usize == &o.h as *const H as usize);
    let a = &o.h as *const H as usize;
    let e = &o.arr[1] as *const P as usize;
    o.h = black_box(H { head: 13, tail: 14 });
    o.arr[1] = mk(3);
    print_bool(a == &o.h as *const H as usize);
    print_bool(e == &o.arr[1] as *const P as usize);
    let r = black_box(&o);
    print_bool(&r.h as *const H == &raw const (*r).h);
    print_bool(&r.h as *const H as usize == a);
    print_bool(&r.arr[0] as *const P as usize == r.arr.as_ptr() as usize);
    print_bool(ptr::eq(&r.arr[1], unsafe { r.arr.as_ptr().add(1) }));
    print_bool(&r.arr[1] as *const P as usize == e);
    print_bool(&r.arr as *const [P; 2] as usize == &r.arr[0] as *const P as usize);
    print_u32(o.n + o.h.head + o.arr[1].x);
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    place_identity();
    place_identity_guarded();
}
