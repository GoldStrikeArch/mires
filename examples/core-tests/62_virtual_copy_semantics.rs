//! `Copy` semantics under scalarization: a copy is a *fresh* object.
//!
//! `a` is handed whole to an exported function — which the flat call ABI may not touch — so it
//! escapes and stays an object; `b` is a copy of it that only ever has one field written and two
//! read, so it is scalarized. Writing `b.high` must not touch `a.high`.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[derive(Clone, Copy)]
struct Pair {
    high: u32,
    low: u32,
}

/// Exported, so the flat call ABI leaves its parameter alone and the argument is passed whole.
#[no_mangle]
pub fn spread(pair: Pair) -> u32 {
    pair.high.wrapping_mul(3).wrapping_add(pair.low)
}

#[inline(never)]
fn run(seed: u32) {
    let a = Pair {
        high: seed,
        low: seed ^ 0xff,
    };
    let mut b = a;
    b.high = 1;
    print_u32(a.high);
    print_u32(b.high);
    print_u32(b.low);
    print_u32(spread(a));
}

#[no_mangle]
pub fn rust_entry() {
    run(7);
}
