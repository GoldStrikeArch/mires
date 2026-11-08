//! A canonical function-pointer Box caller coexists with a prepared direct caller.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;

struct Pair(u32, u32);

#[inline(never)]
fn consume(value: Box<Pair>) -> u32 {
    value.0 + value.1
}

static CALLBACK: fn(Box<Pair>) -> u32 = consume;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(consume(Box::new(Pair(7, 8))));
    print_u32(CALLBACK(Box::new(Pair(9, 10))));
}
