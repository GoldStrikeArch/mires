#![no_std]
#![no_main]
#![allow(warnings)]
#[path="prelude.rs"] mod prelude;
use prelude::*;
// Pre-existing problem 7b, smallest shapes: `__rt.addr` is handed the user's struct itself (a
// pointer to an aggregate is the object) and reads the fields `pr`, `off`, `buf`, `sc`, `es` off it.
use core::hint::black_box;

pub struct P { pub pr: u32 }
pub struct O { pub off: u32 }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let p = black_box(P { pr: 0 });
    print_bool((&p as *const P).is_null()); // native: false
    print_bool((&p as *const P).addr() == 0); // native: false
    let o = black_box(O { off: 5 });
    print_usize((&o as *const O).addr() % 4); // native: 0
    print_bool((&o as *const O).addr() == &o as *const O as usize); // native: true
}
