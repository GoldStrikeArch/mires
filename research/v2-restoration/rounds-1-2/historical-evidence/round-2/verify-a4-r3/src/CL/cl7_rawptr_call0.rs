//! A zero-argument closure reified as a fn pointer, cast to `*const ()` in a constant, transmuted
//! back and called at run time (what `Waker::from_fn_ptr` does with its data pointer).
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

const P0: *const () = (|| 5u32) as fn() -> u32 as *const ();
fn named0() -> u32 { 6 }
const N0: *const () = named0 as fn() -> u32 as *const ();

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let f: fn() -> u32 = unsafe { core::mem::transmute(black_box(P0)) };
    print_u32(f());
    let g: fn() -> u32 = unsafe { core::mem::transmute(black_box(N0)) };
    print_u32(g());
    // run-time control: the same cast made at run time
    let rt: fn() -> u32 = || 7;
    let p = black_box(rt) as *const ();
    let h: fn() -> u32 = unsafe { core::mem::transmute(p) };
    print_u32(h());
}
