//! Verifier fixture (c4_transparent): `#[repr(transparent)]` wrappers, nested: the flag survives; a real field below them clears it
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

#[repr(transparent)]
struct W(P);

#[repr(transparent)]
struct W2(W);

#[repr(transparent)]
struct WR<'a>(&'a P);

#[repr(transparent)]
struct WR2<'a>(WR<'a>);

struct HW<'a> {
    n: u32,
    w: &'a W2,
    wr: WR2<'a>,
}

plain_and_guarded!(transparent, transparent_guarded, 304, {
    let mut w = black_box(W(mk(1)));
    let w_at = &w as *const W as usize;
    let rw: &W = &w;
    let p0: *const P = &rw.0;
    print_bool(ptr::eq(p0, &w.0));
    print_bool(ptr::eq(p0 as *const W, rw));
    print_bool(p0 as usize == address(&w.0));
    print_bool(p0 as usize == w_at);
    print_u32(unsafe { (*p0).x });
    let mut w2 = black_box(W2(W(mk(2))));
    let w2_at = &w2 as *const W2 as usize;
    let rw2: &W2 = &w2;
    let p00: *const P = &rw2.0 .0;
    print_bool(ptr::eq(p00, &w2.0 .0));
    print_bool(ptr::eq(p00 as *const W2, rw2));
    print_bool(p00 as usize == address(&(w2.0).0));
    print_bool(p00 as usize == w2_at);
    print_u32(unsafe { (*p00).y });
    let h = mk(3);
    let wr = black_box(WR(&h));
    let rwr = &wr;
    print_bool(ptr::eq(rwr.0, &h));
    print_bool(rwr.0 as *const P as usize == address(&h));
    print_bool(&rwr.0 as *const &P as usize != rwr.0 as *const P as usize);
    let hw = black_box(HW { n: 1, w: &w2, wr: WR2(WR(&h)) });
    print_bool(&hw.w.0 .0 as *const P as usize == w2_at);
    print_bool(ptr::eq(&(*hw.w).0 .0, &w2.0 .0));
    print_bool(hw.w as *const W2 as usize == w2_at);
    print_bool(ptr::eq(hw.wr.0 .0, &h));
    print_bool(hw.wr.0 .0 as *const P as usize == address(&h));
    print_bool(&hw.wr as *const WR2 as usize != hw.wr.0 .0 as *const P as usize);
    let wrs: [WR; 2] = black_box([WR(&h), WR(&w.0)]);
    print_bool(ptr::eq(wrs[1].0, &w.0));
    print_bool(wrs[black_box(0)].0 as *const P as usize == address(&h));
    print_u32(unsafe { (*(wrs[1].0 as *const P)).x } + hw.n);
    let mw = &mut w;
    let pf: *mut u32 = &mut mw.0.x;
    unsafe { *pf = 11 };
    let mw2 = &mut w2;
    let pg: *mut u32 = &mut mw2.0 .0.y;
    unsafe { *pg = 22 };
    let pm: *mut P = &mut mw2.0 .0;
    unsafe { (*pm).x = 33 };
    print_u32(w.0.x + w2.0 .0.y + w2.0 .0.x);
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    transparent();
    transparent_guarded();
}
