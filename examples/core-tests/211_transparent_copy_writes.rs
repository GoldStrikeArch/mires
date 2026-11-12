//! A written copy of a `#[repr(transparent)]` wrapper of a scalar is a value of its own.
//!
//! Such a wrapper *is* its field, so a copy of it is the very same JavaScript value, and the SSA
//! builder lets both locals reach one SSA value. Writing the copy's field rewrote that shared value:
//! after `let mut b = a; b.0 = 99;` the source `a` read 99 too, whether `a` was a parameter, a
//! local, an element of a static, or an argument the callee was inlined around. Each case prints
//! the source after writing the copy. Every case is wrong on the base linker under
//! `js-profile=release-balanced`; with `JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on`, `local`,
//! `static element` and `loop` are. The expectations come from this file run natively at `-O0` and
//! `-O3`.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

#[repr(transparent)]
#[derive(Clone, Copy)]
struct W(u32);

#[repr(transparent)]
#[derive(Clone, Copy)]
struct Outer(W);

static WS: [W; 3] = [W(10), W(11), W(12)];

#[inline(never)]
fn parameter(a: W) -> u32 {
    let mut b = a;
    b.0 = 99;
    a.0
}

#[inline(never)]
fn nested(a: Outer) -> u32 {
    let mut b = a;
    b.0.0 = 98;
    a.0.0
}

#[inline(never)]
fn read_after(a: W) -> u32 {
    let mut b = a;
    b.0 += 1;
    a.0 * 1000 + b.0
}

#[inline(always)]
fn overwrite(mut c: W) -> u32 {
    c.0 = 77;
    c.0
}

#[inline(never)]
fn inlined_argument(a: W) -> u32 {
    let written = overwrite(a);
    a.0 * 1000 + written
}

#[inline(never)]
fn in_a_loop(a: W, n: u32) -> u32 {
    let mut b = a;
    for i in 0..n {
        b.0 += i;
    }
    a.0 * 1000 + b.0
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_str("parameter");
    print_u32(parameter(black_box(W(5))));

    print_str("local");
    let a = black_box(W(6));
    let mut b = a;
    b.0 = 99;
    print_u32(a.0);

    print_str("static element");
    let mut w = WS[black_box(2)];
    w.0 = 99;
    print_u32(WS[2].0);

    print_str("nested wrapper");
    print_u32(nested(black_box(Outer(W(7)))));

    print_str("read after");
    print_u32(read_after(black_box(W(8))));

    print_str("inlined argument");
    print_u32(inlined_argument(black_box(W(9))));

    print_str("loop");
    print_u32(in_a_loop(black_box(W(3)), black_box(4)));
}
