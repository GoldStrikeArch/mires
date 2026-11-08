//! A struct mutated field by field inside a loop.
//!
//! The accumulator is live across the back edge, so the escape analysis gives the loop header one
//! parameter per field and the loop body builds no object at all.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

struct Pair {
    high: u32,
    low: u32,
}

#[inline(never)]
fn fold(bytes: &[u8; 8]) -> u32 {
    let mut acc = Pair {
        high: 0x9e37_79b9,
        low: 0x85eb_ca6b,
    };
    let mut index = 0;
    while index < 8 {
        let byte = bytes[index];
        acc.high = acc.high.wrapping_mul(0x85eb_ca6b) ^ byte as u32;
        acc.low = acc.low.wrapping_add(acc.high);
        index += 1;
    }
    acc.high ^ acc.low
}

#[no_mangle]
pub fn rust_entry() {
    print_u32(fold(&[1, 2, 3, 4, 5, 6, 7, 8]));
    print_u32(fold(&[9, 8, 7, 6, 5, 4, 3, 2]));
}
