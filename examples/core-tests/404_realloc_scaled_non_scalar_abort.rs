//! Regression test: `realloc` on a scaled non-scalar pointer MUST preserve `.sc`
//! so subsequent byte reads still abort fail-closed as non-scalar storage.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::alloc::{realloc, Layout};
use alloc::boxed::Box;
use core::hint::black_box;

#[repr(C)]
#[allow(dead_code)]
struct Pair {
    a: u32,
    b: u32,
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    unsafe {
        let p = Box::into_raw(Box::new(Pair {
            a: black_box(0x1122_3344),
            b: black_box(0x5566_7788),
        }));
        let l1 = Layout::new::<Pair>();
        let grown = realloc(p as *mut u8, l1, 16);
        let b = *grown;
        js_log_i32(b as i32);
    }
}
