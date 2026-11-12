//! FAIL (round 3): the noop waker's vtable no longer equals itself (`RawWakerVTable: PartialEq`
//! compares its four `fn` pointers, which core builds from closures in a constant).
//! Native (-O0, -O3): true true. Main (ea5604f), all four modes: true true.
//! e2245df, all four modes: false false.
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::task::Waker;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_bool(*Waker::noop().vtable() == *Waker::noop().vtable());
    let cloned = Waker::noop().clone();
    print_bool(*cloned.vtable() == *Waker::noop().vtable());
}
