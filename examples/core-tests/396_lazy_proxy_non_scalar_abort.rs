//! Inline byte dereference over a retyped non-scalar raw allocation must fail closed.
//!
//! Under the lazy proxy-free two-array heap model, `alloc` returns a plain JS array `raw` and
//! `_retype` moves a non-scalar allocation to a separate `typed` array with `raw.length = 0`.
//! Dereferencing `*a.add(0)` through the pre-retype `*mut u8` pointer must abort loudly rather
//! than silently returning `undefined` from the emptied `raw` array.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::alloc::{alloc, dealloc, Layout};
use core::hint::black_box;

#[repr(C)]
#[derive(Clone, Copy)]
struct Pair {
    a: u32,
    b: u32,
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    unsafe {
        let layout = Layout::array::<Pair>(2).unwrap();
        let a = alloc(layout);
        let p = a as *mut Pair;
        *p.add(0) = black_box(Pair { a: 10, b: 20 });
        *p.add(1) = black_box(Pair { a: 30, b: 40 });
        print_u32((*p.add(0)).a);

        // Must fail closed loudly rather than reading `undefined` from the emptied `raw` buffer.
        let byte = black_box(*a.add(0));
        print_u32(u32::from(byte));
        dealloc(a, layout);
    }
}
