//! Aggregates the emitter builds only to take apart again, for the `sroa` pass.
//!
//! Three shapes, on purpose:
//!
//! * `word` builds a `[u8; 4]` at the call, which reaches the byte conversion as a copy of a
//!   fresh literal bound to a parameter: both the copy and the binding are removable;
//! * `mix` takes its argument by value, so each call copies a `Pair` into a temporary the caller
//!   then only reads one field of at a time, which is a local to split;
//! * `spread` hands a `Pair` to a function that keeps it, so the aggregate has to stay whole and
//!   the split has something to decline.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;

#[derive(Clone, Copy)]
struct Pair {
    high: u32,
    low: u32,
}

impl Pair {
    fn mix(&mut self, other: Self) {
        self.high ^= other.low;
        self.low = self.low.wrapping_add(other.high);
    }
}

fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

#[inline(never)]
fn spread(pair: Pair) -> u32 {
    pair.high.wrapping_mul(3).wrapping_add(pair.low)
}

#[no_mangle]
pub fn sroa(seed: u32) -> u32 {
    let bytes = [1_u8, 2, 3, 4, 5, 6, 7, 8];
    let mut accumulated = Pair {
        high: seed,
        low: word(&bytes, 0),
    };
    let step = Pair {
        high: word(&bytes, 4),
        low: seed ^ 0xffff,
    };
    accumulated.mix(step);
    accumulated.mix(step);
    accumulated
        .high
        .wrapping_add(accumulated.low)
        .wrapping_add(spread(accumulated))
}

#[no_mangle]
pub fn rust_entry() {}
