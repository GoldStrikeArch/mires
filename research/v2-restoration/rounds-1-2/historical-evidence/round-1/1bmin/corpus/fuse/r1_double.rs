//! Verifier fixture (r1_double): `*pp`, `**ppp`, a `&&P` field and a `*mut &P` slot pointer
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

struct RR<'a> {
    n: u32,
    rr: &'a &'a P,
}

plain_and_guarded!(double_indirection, double_indirection_guarded, 201, {
    let h = mk(1);
    let h2 = mk(2);
    let a = black_box(R { n: 1, r: &h });
    let pp: &&P = black_box(&a.r);
    let ppp: &&&P = black_box(&pp);
    print_bool(ptr::eq(*pp, &h));
    print_bool(ptr::eq(**ppp, &h));
    print_bool(ptr::eq(*pp, **ppp));
    print_bool(!ptr::eq(**ppp, &h2));
    print_bool(**ppp as *const P as usize == address(&h));
    print_bool(*pp as *const P == &h as *const P);
    print_bool(pp as *const &P as usize != *pp as *const P as usize);
    print_bool(ppp as *const &&P as usize != **ppp as *const P as usize);
    print_bool(ptr::eq(*ppp, &a.r));
    let b = black_box(RR { n: 2, rr: &a.r });
    print_bool(ptr::eq(*b.rr, &h));
    print_bool(*b.rr as *const P as usize == address(&h));
    print_bool(ptr::eq(*b.rr, a.r));
    print_bool(b.rr as *const &P as usize != *b.rr as *const P as usize);
    print_bool(ptr::eq(b.rr, &a.r));
    print_u32((**ppp).y + (*b.rr).x + a.n + b.n);
});

plain_and_guarded!(slot_pointer, slot_pointer_guarded, 202, {
    let h = mk(1);
    let h2 = mk(2);
    let mut local: &P = &h;
    let slot: *mut &P = &mut local;
    unsafe {
        print_bool(ptr::eq(*slot, &h));
        print_bool((*slot) as *const P as usize == address(&h));
        *slot = &h2;
        print_bool(ptr::eq(*slot, &h2));
        print_bool(!ptr::eq(*slot, &h));
        print_bool(slot as usize != (*slot) as *const P as usize);
        print_u32((*slot).y);
    }
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    double_indirection();
    double_indirection_guarded();
    slot_pointer();
    slot_pointer_guarded();
}
