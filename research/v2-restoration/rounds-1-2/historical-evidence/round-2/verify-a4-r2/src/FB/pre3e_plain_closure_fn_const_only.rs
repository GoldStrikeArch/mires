#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
const CF: fn(u32) -> u32 = |x| x + 7;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(CF(1));
}
