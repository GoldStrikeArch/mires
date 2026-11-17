#![no_std]
#![no_main]
extern crate alloc;
#[path = "../../../../examples/core-tests/prelude.rs"] mod prelude;
use prelude::*;
#[no_mangle] pub fn rust_entry() { const F: fn(u32) -> u32 = |x| x + 1; let f = core::hint::black_box(F); print_u32(f(41)); }
