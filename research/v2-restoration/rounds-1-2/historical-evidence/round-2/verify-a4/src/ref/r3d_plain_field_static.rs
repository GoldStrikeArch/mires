#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
struct Pair { a: u32, b: u32 }
static P: Pair = Pair { a: 1, b: 2 };
static FIELD: &u32 = &P.b;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(**black_box(&FIELD));
}
