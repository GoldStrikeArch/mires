//! The native `Vec` carrier's contract, exercised end to end (Phase 4, D5).
//!
//! Everything the `{ buf, len, cap }` NativeVec carrier must preserve when the representation
//! planner commits it: construction with and without an explicit capacity, Rust-visible length
//! and capacity, minimal growth on `push`, `remove` shifting left while keeping the capacity,
//! `extend` moving elements out of an owned source, `as_slice`/`as_mut_slice` borrows (including
//! `swap` through the view), and `clear` keeping the allocation. The expected stdout is
//! mode-independent: the baseline lane, the jir lane and every optimizer mode must print exactly
//! this — the metamorphic gate the carrier rewrite is held to.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::vec::Vec;

fn total(values: &[u32]) -> u32 {
    let mut sum = 0;
    let mut index = 0;
    while index < values.len() {
        sum += values[index];
        index += 1;
    }
    sum
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Construction, emptiness, growth.
    let mut values: Vec<u32> = Vec::new();
    print_usize(values.len());
    print_bool(values.capacity() == 0);
    print_bool(values.is_empty());
    values.push(3);
    values.push(5);
    values.push(8);
    print_usize(values.len());
    print_bool(!values.is_empty());
    print_bool(values.capacity() >= 3);
    print_u32(values[0]);
    print_u32(values[2]);

    // An explicit capacity is preserved until growth demands more.
    let mut sized: Vec<u32> = Vec::with_capacity(8);
    print_usize(sized.capacity());
    sized.push(13);
    print_usize(sized.capacity());
    print_usize(sized.len());

    // `remove` returns the element, shifts the tail left and keeps the capacity.
    let removed = values.remove(1);
    print_u32(removed);
    print_usize(values.len());
    print_u32(values[1]);

    // `extend` of an owned vector moves every element out of the source.
    sized.extend(values);
    print_usize(sized.len());
    print_u32(sized[1]);
    print_u32(sized[2]);

    // Borrows: `as_slice` into an ordinary `&[u32]` function, `as_mut_slice` and a `swap`
    // through the view writing back into the vector.
    print_u32(total(sized.as_slice()));
    let view = sized.as_mut_slice();
    view.swap(0, 2);
    print_u32(sized[0]);
    print_u32(sized[2]);

    // `clear` empties the vector and keeps the block.
    let kept = sized.capacity();
    sized.clear();
    print_usize(sized.len());
    print_bool(sized.capacity() == kept);
}
