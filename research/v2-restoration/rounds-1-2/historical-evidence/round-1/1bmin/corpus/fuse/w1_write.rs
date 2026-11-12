//! Verifier fixture (w1_write): the write side: `*a.r = v` through a `&mut P` read from a field or an element
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

struct RM<'a> {
    n: u32,
    r: &'a mut P,
}

plain_and_guarded!(write_through, write_through_guarded, 402, {
    let mut h = mk(1);
    let mut h2 = mk(2);
    let mut h3 = mk(3);
    {
        let mut a = black_box(RM { n: 1, r: &mut h });
        *a.r = mk(5);
        print_u32(a.r.x + a.n);
    }
    print_u32(h.x);
    print_u32(h.y);
    {
        let mut refs: [&mut P; 2] = black_box([&mut h2, &mut h3]);
        *refs[0] = mk(7);
        *refs[black_box(1)] = P { x: 9, y: 10 };
        print_u32(refs[0].x + refs[1].y);
    }
    print_u32(h2.x);
    print_u32(h3.y);
    let mut local: &mut P = &mut h;
    let slot: &mut &mut P = black_box(&mut local);
    **slot = mk(20);
    print_u32(h.x);
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    write_through();
    write_through_guarded();
}
