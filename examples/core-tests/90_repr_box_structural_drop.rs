//! Native Box drop with compiler-owned structural pointee drop glue.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;

struct Trace(u32);

impl Drop for Trace {
    fn drop(&mut self) {
        print_u32(self.0);
    }
}

struct Holder {
    first: Trace,
    second: Trace,
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let holder = Box::new(Holder {
        first: Trace(10),
        second: Trace(20),
    });
    print_u32(holder.first.0 + holder.second.0);
    drop(holder);
    print_u32(7);
}
