#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
static ARR: [u32; 3] = [100, 200, 300];
static SL: &[u32] = &ARR;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(black_box(&SL)[1]);
    print_usize(SL.len());
}
