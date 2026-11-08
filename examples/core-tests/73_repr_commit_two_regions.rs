//! Two string regions under the representation solver (Phase 4, D3/D4): one legal,
//! cost-positive builder region commits to the mutable native carrier, while a raw-provenance
//! pin keeps the other baseline — and because both are born from the same `String::new`, the
//! pinned region's result crosses the cut through a `Convert` adapter right after the call
//! (the `research/v2/representation_cut.py` scenario as a program).
//!
//! The expected stdout is mode-independent: baseline lane, jir lane, and every optimizer mode
//! must print exactly this — `scripts/repr-report-test.sh` additionally holds `js-opt=off` and
//! `js-opt=report` to identical bytes over this fixture while the sidecar's `repr` block says
//! what `on` commits.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::string::String;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Region one: a mutable builder the planner commits — construction, growth, borrow, length
    // and capacity all stay observable.
    let mut a = String::new();
    a.push_str("alpha");
    a.push('!');
    print_str(&a);
    print_usize(a.len());
    print_bool(a.capacity() >= 6);

    // Region two: `as_mut_vec` projects inside the `String`'s own structure, which pins this
    // string in place. Its producer is the same `String::new` region one committed, so the
    // pinned value arrives through the registry's native-to-baseline adapter — an edge cut,
    // never a region kill.
    let mut b = String::new();
    let inner = unsafe { b.as_mut_vec() };
    print_usize(inner.len());
}
