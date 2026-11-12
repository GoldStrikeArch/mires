#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
static ARR: [u32; 3] = [100, 200, 300];
static FIRST: Option<&u32> = Some(&ARR[0]);
static WHOLE: Option<&[u32; 3]> = Some(&ARR);
static SL: Option<&[u32]> = Some(&ARR);
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(black_box(&FIRST).map_or(0, |v| *v));
    print_u32(black_box(&WHOLE).map_or(0, |v| v[2]));
    print_u32(black_box(&SL).map_or(0, |v| v[1]));
}
