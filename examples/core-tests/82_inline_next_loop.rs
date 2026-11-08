//! An iterator's `next()` in a loop, inlined by the link-time inliner (Phase 5, D6).
//!
//! `Counter::next` is `#[inline(never)]`, so rustc's MIR inliner leaves the call whichever way
//! `JSC_RUST_MIR_INLINE` points — reaching in-loop 0 here is the *link-time* inliner's job. The
//! call sits in a loop and returns an `Option<u32>` (a virtualizable shape), so the O group
//! splices the body into `sum`; the discriminant jump-threading then splits the `Some`/`None`
//! merge per variant and the escape analysis scalarizes the `Some` — the Murmur mechanism. The
//! `.alloc` gate holds the loop at zero materializations and copies.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

struct Counter {
    at: u32,
    end: u32,
}

impl Counter {
    #[inline(never)]
    fn next(&mut self) -> Option<u32> {
        if self.at < self.end {
            let v = self.at;
            self.at += 1;
            Some(v)
        } else {
            None
        }
    }
}

#[inline(never)]
fn sum(seed: u32, end: u32) -> u32 {
    let mut counter = Counter { at: seed, end };
    let mut acc = 0u32;
    while let Some(v) = counter.next() {
        acc = acc.wrapping_mul(31) ^ v;
    }
    acc
}

#[no_mangle]
pub fn rust_entry() {
    print_u32(sum(0, 16));
    print_u32(sum(7, 7));
    print_u32(sum(0xfff0, 0x1_0000));
}
