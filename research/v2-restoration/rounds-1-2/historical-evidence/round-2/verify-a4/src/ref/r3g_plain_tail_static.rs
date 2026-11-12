#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
static ARR: [u32; 3] = [100, 200, 300];
static TAIL: &[u32] = ARR.split_at(1).1;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(black_box(&TAIL)[0]);
}
