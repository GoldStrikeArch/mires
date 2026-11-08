//! A closure environment built inside a loop and called there.
//!
//! The environment is an aggregate like any other: `mix` captures two `u32` by move, and with the
//! closure body inlined into the caller its only uses are field reads. The escape analysis then
//! never builds the environment array, and the loop body allocates nothing. Without inlining the
//! call passes the environment whole, so it escapes and the loop keeps its array — the printed
//! numbers are the same either way.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[inline(never)]
fn drive(seed: u32) -> u32 {
    let high = seed;
    let low = seed ^ 0xff;
    let mut acc = 0x9e37_79b9u32;
    let mut index = 0;
    while index < 4 {
        let mix = move |x: u32| x.wrapping_mul(high) ^ low;
        acc = mix(acc).wrapping_add(mix(index));
        index += 1;
    }
    acc
}

#[no_mangle]
pub fn rust_entry() {
    print_u32(drive(3));
    print_u32(drive(0x9000_0000));
}
