//! Control: pointer equality of fn pointers made by the RUN-TIME closure coercion (no constants).
#![no_std]
#![no_main]
#![allow(unpredictable_function_pointer_comparisons)]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

#[inline(never)]
fn make() -> fn() -> u32 { || 5 }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let c = || 6u32;
    let f: fn() -> u32 = c;
    let g: fn() -> u32 = c;
    print_bool(black_box(f) == black_box(g));
    print_bool(black_box(f) == black_box(f));
    print_bool(black_box(make()) == black_box(make()));
    print_u32(f() + g() + make()());
}
