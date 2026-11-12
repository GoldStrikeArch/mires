//! Verifier fixture (r5_dyn): a `&dyn Trait` receiver cast to `*const ()` against the value's address
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

trait Shape {
    fn area(&self) -> u32;
    fn me(&self) -> *const ();
    fn is(&self, other: *const ()) -> bool;
}

impl Shape for P {
    fn area(&self) -> u32 {
        self.x * self.y
    }
    #[inline(never)]
    fn me(&self) -> *const () {
        self as *const P as *const ()
    }
    #[inline(never)]
    fn is(&self, other: *const ()) -> bool {
        ptr::eq(self as *const P as *const (), other)
    }
}

struct D<'a> {
    n: u32,
    d: &'a dyn Shape,
}

plain_and_guarded!(dyn_receiver, dyn_receiver_guarded, 207, {
    let h = mk(2);
    let h2 = mk(3);
    let d: &dyn Shape = black_box(&h);
    print_u32(d.area());
    print_bool(d as *const dyn Shape as *const () == &h as *const P as *const ());
    print_bool(ptr::addr_eq(d, &h));
    print_bool(d as *const dyn Shape as *const () as usize == address(&h));
    print_bool(d.me() == &h as *const P as *const ());
    print_bool(d.me() as usize == address(&h));
    print_bool(d.is(&h as *const P as *const ()));
    print_bool(d.me() == d as *const dyn Shape as *const ());
    let a = black_box(R { n: 1, r: &h2 });
    let d2: &dyn Shape = a.r;
    print_bool(ptr::addr_eq(d2, &h2));
    print_bool(d2 as *const dyn Shape as *const () as usize == address(&h2));
    print_bool(d2.me() == a.r as *const P as *const ());
    print_bool(d2.is(a.r as *const P as *const ()));
    print_bool(!d2.is(d.me()));
    print_bool(!ptr::addr_eq(d2, d));
    print_u32(d2.area() + a.n);
    let c = black_box(D { n: 4, d: &h });
    print_bool(ptr::addr_eq(c.d, &h));
    print_bool(c.d as *const dyn Shape as *const () == &h as *const P as *const ());
    print_bool(c.d.me() as usize == address(&h));
    print_bool(&c.d as *const &dyn Shape as *const () as usize != c.d as *const dyn Shape as *const () as usize);
    let ds: [&dyn Shape; 2] = black_box([&h, a.r]);
    print_bool(ptr::addr_eq(ds[1], &h2));
    print_bool(ds[0] as *const dyn Shape as *const () as usize == address(&h));
    print_bool(ds[black_box(1)].me() == &h2 as *const P as *const ());
    print_u32(ds[0].area() + ds[1].area() + c.n);
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    dyn_receiver();
    dyn_receiver_guarded();
}
