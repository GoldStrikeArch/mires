//! An `Option` merge with one opaque source, split into its payload (Phase 5, W5g).
//!
//! `refill` returns an `Option<u32>` and stays a real call: it has two callers, so neither the
//! single-caller nor the small-callee criterion admits it, and rustc's MIR inliner is not what
//! this fixture tests. In the loop its result merges with an `Option` built locally, and the
//! merged value is only ever read as the `Some` payload. The discriminant jump-threading sends
//! the local creation straight to its arm, but the merge parameter still has the call's `Some`
//! arm as a predecessor — not a node the escape analysis can virtualize, so the local `Some`
//! would be materialized on its edge every iteration. Payload merge splitting dissolves the
//! merge instead: the parameter becomes the payload, the call edge reads `._0` on its own edge
//! after the switch proved it, and the local creation site dies unused. This is the Murmur
//! kernel's `Skip::next`-calls-`nth` shape (`x64hash128_bytes`, the skip loop). The `.alloc`
//! gate holds the loop at zero materializations and copies.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

struct Feed {
    at: u32,
    end: u32,
    stride: u32,
}

#[inline(never)]
fn refill(feed: &mut Feed) -> Option<u32> {
    if feed.at < feed.end {
        let value = feed.at.wrapping_mul(feed.stride) ^ (feed.end - feed.at);
        feed.at += 1;
        Some(value.rotate_left(feed.stride % 7))
    } else {
        None
    }
}

#[inline(never)]
fn mix(seed: u32, end: u32) -> u32 {
    let mut feed = Feed {
        at: seed,
        end,
        stride: 31,
    };
    let mut acc = 0u32;
    let mut turn = 0u32;
    loop {
        // Odd turns build the option locally, even turns take the callee's: the merge the
        // splitting has to dissolve, with the call keeping one predecessor opaque.
        turn = turn.wrapping_add(1);
        let item = if turn & 1 == 1 {
            Some(acc >> 3)
        } else {
            refill(&mut feed)
        };
        match item {
            Some(value) => acc = acc.wrapping_mul(31) ^ value,
            None => break,
        }
        if turn > 64 {
            break;
        }
    }
    acc
}

#[no_mangle]
pub fn rust_entry() {
    print_u32(mix(0, 16));
    // The second caller, which is what keeps `refill` a real call for the loop above.
    let mut feed = Feed {
        at: 3,
        end: 5,
        stride: 7,
    };
    let mut sum = 0u32;
    while let Some(value) = refill(&mut feed) {
        sum = sum.wrapping_add(value);
    }
    print_u32(sum);
}
