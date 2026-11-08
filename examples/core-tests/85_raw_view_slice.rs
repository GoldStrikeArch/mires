//! The W5f raw-provenance whitelist and its guard (Phase 5).
//!
//! `whitelisted_box_view` is the one shape the FingerprintJS autopsy found behind every
//! `legality.raw-provenance` candidate: a fresh allocation whose raw pointer is only ever
//! re-typed, written through with candidate-free data, and transmuted into a candidate `Box`.
//! The value-flow whitelist (`analysis/valueflow.rs::tame_fresh_box_views`) proves that chain
//! observation-free, so the box candidate no longer declines `legality.raw-provenance` — it
//! declines `legality.user-defined-drop`, its true class (every `Box` owns drop glue).
//!
//! `guarded_box_views` must STAY pinned: it compares the two allocations' addresses before
//! forming the boxes, and an address comparison observes what a carrier change would alter, so
//! both candidates keep the exact decline `legality.raw-provenance`.
//!
//! `scripts/raw-view-test.sh` asserts both sidecar facts and that stdout is byte-identical in
//! every optimizer mode — a representation decline is a planning decision, never a behavior
//! change.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::alloc::{alloc, Layout};
use alloc::boxed::Box;

/// The whitelisted shape: alloc, re-type, init through the raw view, one box transmute.
#[inline(never)]
fn whitelisted_box_view() -> u32 {
    unsafe {
        let raw = alloc(Layout::new::<u32>()) as *mut u32;
        *raw = 41;
        let boxed: Box<u32> = core::mem::transmute(raw);
        *boxed + 1
    }
}

/// The guard: the same construction, but the addresses are compared first. The comparison is an
/// observation, so the whitelist refuses both allocations and the raw-provenance pin stays.
#[inline(never)]
fn guarded_box_views() -> u32 {
    unsafe {
        let first = alloc(Layout::new::<u32>()) as *mut u32;
        let second = alloc(Layout::new::<u32>()) as *mut u32;
        *first = 5;
        *second = 9;
        let distinct = (first != second) as u32;
        let a: Box<u32> = core::mem::transmute(first);
        let b: Box<u32> = core::mem::transmute(second);
        *a + *b + distinct
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(whitelisted_box_view());
    print_u32(guarded_box_views());
}
