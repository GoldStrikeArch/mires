//! Host-delegated `core::fmt::float` fallback through `Formatter::pad_formatted_parts`.
//!
//! Exercises custom `impl fmt::Display`, `Debug`, `LowerExp`, `UpperExp`, width, alignment,
//! fill, sign-aware zero padding, and huge precision (`{:.50}`) on `f64` and `f32` — proving
//! `flt2dec`/Grisu/Dragon4 stay completely absent even when `Formatter` is used.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::format;
use core::fmt;
use core::hint::black_box;

struct Pair(f64, f32);

impl fmt::Display for Pair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:10.3}|{:010.3}|{:+010.3}|{:<10.2}|{:^10.2}|{:e}|{:E}|{:.2e}|{:.3E}|{:?}|{:?}",
            self.0, self.0, self.0, self.0, self.0, self.0, self.0, self.0, self.0, self.0, self.1
        )
    }
}

#[no_mangle]
pub fn rust_entry() {
    for &(a, b) in &[
        (0.0f64, 0.0f32),
        (-0.0f64, -0.0f32),
        (f64::NAN, f32::NAN),
        (f64::INFINITY, f32::INFINITY),
        (f64::NEG_INFINITY, f32::NEG_INFINITY),
        (1.5, 0.1),
        (-12.3456, 0.0001),
        (9.995, 1e-5),
        (1e25, 1e16),
        (f64::from_bits(1), f32::from_bits(1)),
    ] {
        let p = black_box(Pair(a, b));
        print_str(&format!("{}", p));
    }
    print_str(&format!("{:.50}", black_box(0.5f64)));
    print_str(&format!("{:.200}", black_box(0.125f64)));
    print_str(&format!("{:.2?}|{:.0e}", black_box(9.95f64), black_box(9.95f64)));
    print_str(&format!("{:e}|{:E}|{:.2e}", black_box(1234.5f32), black_box(f32::MAX), black_box(9.995f32)));
}
