//! A closure reified as a fn pointer and cast to `*const ()` inside a constant; only safe
//! observations of the pointer (null test, self-equality).
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

const P0: *const () = (|| 5u32) as fn() -> u32 as *const ();
struct SyncPtr(*const ());
unsafe impl Sync for SyncPtr {}
static SP: SyncPtr = SyncPtr((|x: u32| x + 1) as fn(u32) -> u32 as *const ());

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_bool(P0.is_null());
    print_bool(black_box(P0) == black_box(P0));
    print_bool(SP.0.is_null());
    print_bool(black_box(SP.0) == SP.0);
}
