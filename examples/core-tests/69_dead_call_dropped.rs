//! An item nothing calls once the lane has folded the call away.
//!
//! `pick` builds an `Option` and immediately matches it. The monomorphization collector sees both
//! arms — the discriminant is not a constant at collection time — so `cold` is codegenned and the
//! baseline prints it. With `js-opt=on` the escape analysis takes the `Option` apart, its
//! discriminant becomes a constant, and the dead-code pass folds the match to its `Some` arm: the
//! only reference to `cold` in the whole program is gone.
//!
//! The item table still defines `cold`, and the reachability walk that decided what this program
//! prints ran before the lane folded anything. So this fixture is the driver's re-walk: without
//! it the program still carries `cold`, an item nothing calls.
//!
//! Both modes print the same number; what the fold changes is what is in the file.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

/// Reachable only from the arm the fold deletes.
#[inline(never)]
fn cold(x: u32) -> u32 {
    x.wrapping_mul(9973).wrapping_add(7)
}

#[inline(never)]
fn pick(v: u32) -> u32 {
    let opt = Some(v);
    match opt {
        Some(x) => x.wrapping_add(1),
        None => cold(v),
    }
}

#[no_mangle]
pub fn rust_entry() {
    print_u32(pick(41));
}
