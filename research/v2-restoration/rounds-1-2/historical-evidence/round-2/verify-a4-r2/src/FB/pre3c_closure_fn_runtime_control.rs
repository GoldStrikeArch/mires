#![no_std]
#![no_main]
#![allow(dead_code, unused_imports)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let f: fn(u32) -> u32 = |x| x + 1; print_u32(black_box(f)(1));
}
