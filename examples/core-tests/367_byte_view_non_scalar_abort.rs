//! Non-primitive aggregate storage viewed as a byte slice must fail closed loudly (MIRES-007).
//!
//! Primitive integer, float, and `char` buffers support aliasing little-endian byte views,
//! whereas aggregate storage (`#[repr(C)] struct Pair { a: u32, b: u32 }`) holds JavaScript
//! objects rather than flat bytes. Building a non-empty byte slice over such storage via
//! `slice::from_raw_parts` (or `align_to::<u8>()`) must abort with a registered runtime error
//! rather than silently returning JavaScript objects or `undefined`.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::slice;

#[repr(C)]
struct Pair {
    a: u32,
    b: u32,
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    let pairs = black_box([Pair { a: 1, b: 2 }, Pair { a: 3, b: 4 }]);
    print_usize(pairs.len());

    // Must fail closed with a registered abort rather than silently exposing JS objects.
    let bytes = unsafe { slice::from_raw_parts(pairs.as_ptr().cast::<u8>(), 16) };

    // Unreachable: `slice_from_parts` aborts above.
    print_i32(i32::from(bytes[0]));
}
