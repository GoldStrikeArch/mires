//! Objects that reach a merge with mixed states, materialized on the offending edge.
//!
//! Two shapes sit here. `merge_pick` merges a caller-supplied `Pair` with one built locally: the
//! merge parameter is a real object (its predecessors disagree), but the built pair stays
//! scalar — `peek` reads it as fields — and becomes an object only on the one edge that needs
//! it. `drain` carries an `Option<u32>` around a loop whose variants disagree between the entry
//! and the latch: the header parameter is real, and each arm's option is built on its own edge
//! rather than escaping everywhere.

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
fn observe(p: &Pair) -> u32 {
    p.high ^ p.low
}

/// The condition is a value the caller computed, so nothing folds the branch away.
#[inline(never)]
fn merge_pick(seed: u32, given: Pair) -> u32 {
    let built = Pair {
        high: seed.wrapping_mul(3),
        low: seed ^ 5,
    };
    // A read that never needs the object: it stays two locals.
    let peek = built.high.wrapping_add(built.low);
    let chosen = if seed > 0x8000_0000 { given } else { built };
    observe(&chosen).wrapping_add(peek)
}

/// A loop-carried option whose variant differs between the entry edge and the latch.
#[inline(never)]
fn drain(limit: u32) -> u32 {
    let mut state = Some(limit);
    let mut acc = 0u32;
    loop {
        match state {
            Some(n) => {
                acc = acc.wrapping_mul(31).wrapping_add(n);
                state = if n == 0 { None } else { Some(n - 1) };
            }
            None => break,
        }
    }
    acc
}

#[no_mangle]
pub fn rust_entry() {
    print_u32(merge_pick(3, Pair { high: 7, low: 9 }));
    print_u32(merge_pick(0x9000_0000, Pair { high: 7, low: 9 }));
    print_u32(drain(5));
}
