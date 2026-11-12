//! Verifier fixture (r6_closure): closures capturing a reference field and returning its address
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

#[inline(never)]
fn call_dyn(f: &dyn Fn() -> usize) -> usize {
    f()
}

#[inline(never)]
fn call_with<F: Fn(&R) -> *const P>(f: F, a: &R) -> *const P {
    f(a)
}

plain_and_guarded!(closure_capture, closure_capture_guarded, 208, {
    let h = mk(1);
    let h2 = mk(2);
    let a = black_box(R { n: 1, r: &h });
    let f = || a.r as *const P as usize;
    print_bool(f() == address(&h));
    let r = a.r;
    let g = move || r as *const P;
    print_bool(ptr::eq(g(), &h));
    print_bool(g() == a.r as *const P);
    let hh = || &a.r as *const &P as usize;
    print_bool(hh() != f());
    let k = |q: &R| q.r as *const P as usize;
    print_bool(k(&a) == address(&h));
    print_bool(k(&black_box(R { n: 2, r: &h2 })) == address(&h2));
    let m = |q: &R| ptr::eq(q.r, &h);
    print_bool(m(&a));
    print_bool(!m(&R { n: 3, r: &h2 }));
    print_bool(call_dyn(&f) == address(&h));
    print_bool(call_dyn(&|| a.r as *const P as usize) == f());
    print_bool(ptr::eq(call_with(|q| q.r as *const P, &a), &h));
    print_bool(call_with(|q| ptr::from_ref(q.r), &a) == g());
    let b = black_box(R { n: 5, r: &h2 });
    let pick = move || if b.n == 5 { b.r as *const P } else { ptr::null() };
    print_bool(ptr::eq(pick(), &h2));
    print_u32(a.n);
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    closure_capture();
    closure_capture_guarded();
}
