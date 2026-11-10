//! Capacity across a native->baseline string cut edge (Phase 4, D6).
//!
//! `rust_entry`'s builder is committed native (`with_capacity` + `push_str` force the
//! `{ text, cap, len }` carrier), while `sink` is pinned baseline by raw provenance
//! (`as_mut_vec`). The move into `sink` is therefore a cut edge whose Convert adapter
//! must carry the builder's Rust-visible capacity into the baseline cap leaf: a moved
//! `String` keeps its capacity, so both sides print 64, in every mode.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::string::String;

fn sink(mut b: String) {
    // Observe len and capacity on the baseline side of the cut.
    print_usize(b.len());
    print_usize(b.capacity());
    // Raw-provenance pin: keeps this region baseline.
    let inner = unsafe { b.as_mut_vec() };
    print_usize(inner.len());
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut a = String::with_capacity(64);
    a.push_str("hi");
    print_usize(a.capacity());
    sink(a);
}
