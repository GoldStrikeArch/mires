//! Verifier fixture (c2_pointee): surviving the inverse: `&*(*r).next` is the box of the pointee, not the field slot
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

struct Inner {
    f: u32,
    g: u32,
}

struct Nested<'a> {
    k: u32,
    link: &'a P,
}

struct Outer<'a> {
    inner: Inner,
    arr: [u32; 4],
    next: &'a P,
    parr: [P; 2],
    nested: Nested<'a>,
}

plain_and_guarded!(pointee, pointee_guarded, 302, {
    let h = mk(1);
    let h3 = mk(3);
    let mut o = black_box(Outer {
        inner: Inner { f: 1, g: 2 },
        arr: [0, 1, 2, 3],
        next: &h,
        parr: [mk(5), mk(6)],
        nested: Nested { k: 4, link: &h3 },
    });
    let r = &mut o;
    let pn: *const P = &*(*r).next;
    print_bool(ptr::eq(pn, &h));
    print_bool(pn as usize == address(&h));
    print_bool(pn as usize != &r.next as *const &P as usize);
    print_u32(unsafe { (*pn).y });
    let pl: *const P = &*(*r).nested.link;
    print_bool(ptr::eq(pl, &h3));
    print_bool(pl as usize == address(&h3));
    print_bool(pl as usize != &r.nested.link as *const &P as usize);
    print_bool(pl != pn);
    r.next = black_box(&h3);
    let pn2: *const P = &*(*r).next;
    print_bool(pn2 == pl);
    print_bool(ptr::eq(pn2, &h3));
    print_bool(pn2 != pn);
    print_u32(unsafe { (*pn).x + (*pn2).x });
    let po: *const Outer = &*r;
    print_u32(unsafe { (*po).arr[2] });
    print_bool(ptr::eq(po, &o));
    print_bool(po as usize == &o as *const Outer as usize);
    print_u32(o.inner.f + o.next.x);
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    pointee();
    pointee_guarded();
}
