//! The allocator shim, which is the one thing in a program that no crate has a body for.
//!
//! Compiled against the real `core` and `alloc` (`17_alloc_shim.core`), and filtered down to the
//! shim alone (`17_alloc_shim.skip`): what is pinned is the four `__rust_*` functions the backend
//! synthesizes, the marker beside them, and the shape of the `Alignment` unwrap in each. See
//! `CONTRACT.md`, "Allocation".

#![no_std]
#![no_main]

extern crate alloc;

#[path = "../core-tests/prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let boxed = Box::new(41i32);
    print_i32(*boxed + 1);
}
