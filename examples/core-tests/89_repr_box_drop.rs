//! Default-allocator Box drop whose allocator and address are unobservable.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;

struct Pair {
    left: u32,
    right: u32,
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let pair = Box::new(Pair { left: 20, right: 22 });
    print_u32(pair.left + pair.right);
    drop(pair);
    print_u32(7);
}
