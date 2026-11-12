//! Verifier fixture (r10_transmute): probe: a reference transmuted to a raw pointer (no `ref_to_raw` on that route)
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

plain_and_guarded!(transmuted, transmuted_guarded, 215, {
    let h = mk(1);
    let a = black_box(R { n: 1, r: &h });
    let p: *const P = unsafe { core::mem::transmute::<&P, *const P>(a.r) };
    print_bool(p == &h as *const P);
    print_bool(ptr::eq(p, &h));
    print_u32(unsafe { (*p).x } + a.n);
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    transmuted();
    transmuted_guarded();
}
