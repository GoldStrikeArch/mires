//! An `Option`-shaped merge switched immediately: threading makes it variant-per-path.
//!
//! `select` builds an option under a runtime condition and matches it at once, inside a loop.
//! Without threading the two creation sites merge into one block parameter whose variant is
//! unknown, the option escapes the escape analysis, and the loop body builds a fresh object per
//! iteration. The discriminant jump-threading retargets each creation edge straight to its arm,
//! after which each site has one variant and the escape analysis scalarizes both: the loop body
//! materializes nothing. The `.alloc` gate holds `in_loop` at 0.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[inline(never)]
fn select(seed: u32, mask: u32) -> u32 {
    let mut acc = 0u32;
    let mut i = 0u32;
    while i < 16 {
        let option = if (seed >> i) & 1 == 1 {
            Some((seed.wrapping_add(i), mask ^ i))
        } else {
            None
        };
        acc = match option {
            Some((high, low)) => acc.wrapping_mul(31) ^ high.wrapping_add(low),
            None => acc.rotate_left(7),
        };
        i += 1;
    }
    acc
}

#[no_mangle]
pub fn rust_entry() {
    print_u32(select(0b1010_1100, 0x00ff_00ff));
    print_u32(select(0, 1));
    print_u32(select(u32::MAX, 0x1234_5678));
}
