//! The flat direct-call ABI: an aggregate argument passed as its fields, and the spill register a
//! `&mut` one is copied back through.
//!
//! The plan runs with `sroa`, so the shapes below are what `js-lane=jir js-opt=on` prints; every
//! other mode prints the baseline call and the same numbers. `js-jir-flat-abi=off` is the same
//! program again with the plan bisected out.
//!
//! Three shapes, on purpose:
//!
//! * `step` takes its accumulator by `&mut` and its step by value, and mutates both fields. Both
//!   parameters flatten, and the two fields the callee writes back leave as the return value and
//!   as `$r1`, so the loop below never builds a `Pair` at all;
//! * `peek` takes a shared reference and only reads, which is the copy-in half with no copy-back;
//! * `keep` hands its whole parameter to an exported function, whose calling convention this
//!   program does not own. `keep`'s parameter therefore escapes and both stay in the baseline
//!   shape, which is what the plan declining one item has to look like.

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

#[inline(never)]
fn step(acc: &mut Pair, k: Pair) {
    acc.high = acc.high.wrapping_mul(5).wrapping_add(k.high);
    acc.low = (acc.low ^ k.low).wrapping_add(acc.high);
}

#[inline(never)]
fn peek(p: &Pair) -> u32 {
    p.high ^ p.low
}

/// Exported, so the link step may not change how it is called.
#[no_mangle]
pub fn spread(p: Pair) -> u32 {
    p.high.wrapping_mul(3).wrapping_add(p.low)
}

#[inline(never)]
fn keep(p: Pair) -> u32 {
    spread(p)
}

#[no_mangle]
pub fn flat_abi_spill(seed: u32) -> u32 {
    let mut acc = Pair {
        high: seed,
        low: seed ^ 0xffff,
    };
    let mut i = 0_u32;
    while i < 4 {
        step(
            &mut acc,
            Pair {
                high: i,
                low: i.wrapping_mul(7),
            },
        );
        i += 1;
    }
    peek(&acc).wrapping_add(keep(acc))
}

#[no_mangle]
pub fn rust_entry() {
    print_u32(flat_abi_spill(1));
    print_u32(flat_abi_spill(0xdead_beef));
}
