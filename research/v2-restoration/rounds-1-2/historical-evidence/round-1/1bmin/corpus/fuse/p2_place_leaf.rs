//! Verifier fixture (p2_place_leaf): aggregate-place relations asked inside call-free leaf bodies (the bodies that fuse under js-cleanup=on)
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

#[inline(never)]
fn nested_eq(o: &Outer) -> bool {
    &o.h as *const H == &raw const o.h
}

#[inline(never)]
fn elem_eq(o: &Outer) -> bool {
    &o.arr[1] as *const P == &raw const o.arr[1]
}

#[inline(never)]
fn elem_distance(o: &Outer) -> usize {
    (&o.arr[1] as *const P as usize) - (&o.arr[0] as *const P as usize)
}

#[inline(never)]
fn nested_addr(o: &Outer) -> usize {
    &o.h as *const H as usize
}

#[inline(never)]
fn nested_raw_addr(o: &Outer) -> usize {
    &raw const o.h as usize
}

#[inline(never)]
fn elem_addr(v: &[P], i: usize) -> usize {
    &v[i] as *const P as usize
}

#[inline(never)]
fn elem_raw_addr(v: &[P], i: usize) -> usize {
    &raw const v[i] as usize
}

plain_and_guarded!(place_leaves, place_leaves_guarded, 405, {
    let o = black_box(Outer { n: 17, h: H { head: 11, tail: 12 }, arr: [mk(1), mk(2)] });
    print_bool(nested_eq(&o));
    print_bool(elem_eq(&o));
    print_bool(elem_distance(&o) == core::mem::size_of::<P>());
    print_bool(nested_addr(&o) == nested_raw_addr(&o));
    print_bool(elem_addr(&o.arr, 1) == elem_raw_addr(&o.arr, 1));
    print_bool(elem_addr(&o.arr, 0) < elem_addr(&o.arr, 1));
    print_bool(elem_addr(&o.arr, 1) - elem_addr(&o.arr, 0) == 8);
    print_u32(o.n + o.h.head + o.h.tail);
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    place_leaves();
    place_leaves_guarded();
}
