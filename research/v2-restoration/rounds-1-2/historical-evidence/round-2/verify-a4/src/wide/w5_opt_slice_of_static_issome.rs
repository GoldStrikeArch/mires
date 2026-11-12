#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
static ARR: [u32; 3] = [100, 200, 300];
static SL: Option<&[u32]> = Some(&ARR);
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_bool(black_box(&SL).is_some());
}
