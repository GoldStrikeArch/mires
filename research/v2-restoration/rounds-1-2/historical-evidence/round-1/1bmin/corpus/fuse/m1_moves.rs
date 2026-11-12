//! Verifier fixture (m1_moves): moving references between elements must not touch the pointees
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

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

plain_and_guarded!(moves, moves_guarded, 403, {
    let h = mk(1);
    let h2 = mk(2);
    let h3 = mk(3);
    let mut refs: [&P; 2] = black_box([&h, &h2]);
    refs.swap(0, 1);
    print_u32(h.x);
    print_u32(h2.x);
    print_u32(refs[0].x);
    print_bool(ptr::eq(refs[0], &h2));
    let mut v: Vec<&P> = vec![&h, &h2, &h3];
    v.reverse();
    print_u32(h.x + h.y);
    print_u32(h3.x + h3.y);
    print_bool(ptr::eq(v[0], &h3));
    v.sort_by_key(|r| r.x);
    print_u32(v[0].x);
    print_u32(h.x);
    let mut w: Vec<&P> = vec![&h3, &h, &h2];
    w.sort_by_key(|r| *r as *const P as usize);
    let a = w[0] as *const P as usize;
    let b = w[1] as *const P as usize;
    let c = w[2] as *const P as usize;
    print_bool(a < b && b < c);
    print_u32(h.x + h2.x + h3.x);
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    moves();
    moves_guarded();
}
