//! `write_bytes` (including zero fill) over a non-scalar byte view must fail closed (MIRES-008).
//!
//! Filling a `*mut u8` byte view of `struct Pair { a: u32, b: u32 }` with `0` has no zero
//! factory for `Pair`: writing the JavaScript number `0` into the `Pair` slot would silently
//! corrupt the struct element into `0` (reading `undefined` for its fields). Both the scaled
//! `p.cast::<u8>()` view and a raw retyped allocation pointer must abort loudly instead.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::alloc::{alloc, dealloc, Layout};
use core::hint::black_box;
use core::ptr;

#[repr(C)]
#[derive(Clone, Copy)]
struct Pair {
    a: u32,
    b: u32,
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    unsafe {
        let layout = Layout::array::<Pair>(4).unwrap();
        let a = alloc(layout);
        let p = a as *mut Pair;
        *p.add(0) = black_box(Pair { a: 1, b: 2 });
        *p.add(1) = black_box(Pair { a: 3, b: 4 });
        print_u32((*p.add(1)).a);

        // Must fail closed loudly rather than silently overwriting raw[1] with JS number `0`.
        ptr::write_bytes(p.cast::<u8>().add(8), 0, 8);

        // Unreachable: `write_bytes` aborts above.
        print_u32((*p.add(1)).a);
        dealloc(a, layout);
    }
}
