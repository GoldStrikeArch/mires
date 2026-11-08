//! Phase 12D's opt-in exact numeric storage and its first hard declines.
//!
//! `exact` has only private indexed consumers. `laned` feeds short-lived slices exclusively into
//! a selected scalar slice ABI. `observed` reads Rust-visible capacity and `retained` inspects a
//! materialized slice; those two must stay on ordinary NativeVec storage even when exact typed
//! storage is enabled.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::vec::Vec;

fn sum(values: &[u32]) -> u32 {
    values[0] + values[1]
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let seed = [5_u32, 7];
    print_u32(
        sum(&seed)
            + sum(&seed)
            + sum(&seed)
            + sum(&seed)
            + sum(&seed)
            + sum(&seed)
            + sum(&seed)
            + sum(&seed),
    );

    let mut exact: Vec<u32> = Vec::with_capacity(2_u8 as usize);
    exact.push(u32::MAX);
    exact.push(7);
    print_u32(exact[0]);
    print_u32(exact[1]);
    print_usize(exact.len());

    let mut observed: Vec<u32> = Vec::with_capacity(4);
    print_usize(observed.capacity());
    observed.push(11);
    print_u32(observed[0]);

    let mut laned: Vec<u32> = Vec::with_capacity(2);
    laned.push(19);
    laned.push(23);
    print_u32(
        sum(laned.as_slice())
            + sum(laned.as_slice())
            + sum(laned.as_slice())
            + sum(laned.as_slice()),
    );

    let mut retained: Vec<u32> = Vec::with_capacity(2);
    retained.push(13);
    retained.push(17);
    let slice = retained.as_slice();
    print_u32(slice[0] + slice[1]);
}
