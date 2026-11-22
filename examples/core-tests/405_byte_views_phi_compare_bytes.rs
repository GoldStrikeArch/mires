//! Regression test: phi joining a wide-scalar-scaled `*const u8` with an ordinary `[u8; 4]`
//! pointer feeding `compare_bytes == 0` must NOT elide `scale_scalar` (`propagate_scalar_origins`
//! must intersect all incoming edges, not union only `scalar_scales`).

#![no_std]
#![no_main]
#![feature(core_intrinsics)]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    let bytes: [u8; 4] = black_box([0x44, 0x33, 0x22, 0x11]);
    let x: u32 = black_box(0x1122_3344);
    let y: u32 = black_box(0x1122_3344);
    let p: *const u8 = if black_box(false) {
        (&x as *const u32) as *const u8
    } else {
        bytes.as_ptr()
    };
    let q: *const u8 = (&y as *const u32) as *const u8;
    let eq_false = unsafe { core::intrinsics::compare_bytes(p, q, 4) == 0 };

    let p2: *const u8 = if black_box(true) {
        (&x as *const u32) as *const u8
    } else {
        bytes.as_ptr()
    };
    let eq_true = unsafe { core::intrinsics::compare_bytes(p2, q, 4) == 0 };

    unsafe {
        js_log_bool(eq_false);
        js_log_bool(eq_true);
    }
}
