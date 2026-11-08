//! A struct built in both arms of a branch and then only read field by field.
//!
//! The two `Pair`s meet at a block parameter, which the escape analysis turns into one parameter
//! per field: neither object is ever built, and the emitted function mentions no object literal.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

struct Pair {
    high: u32,
    low: u32,
}

/// The condition is a value the caller computed, so nothing folds the branch away.
#[inline(never)]
fn sum(seed: u32) -> u32 {
    let pair = if seed > 0x8000_0000 {
        Pair {
            high: seed,
            low: 1,
        }
    } else {
        Pair {
            high: 1,
            low: seed,
        }
    };
    pair.high.wrapping_add(pair.low)
}

#[inline(never)]
fn scramble(x: u32) -> u32 {
    x.wrapping_mul(2_654_435_761)
}

#[no_mangle]
pub fn rust_entry() {
    print_u32(sum(scramble(3)));
    print_u32(sum(scramble(10)));
}
