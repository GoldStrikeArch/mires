//! Fail-closed `fmt_kernel` boundaries: unsupported format specifiers (custom fill, explicit
//! alignment, dynamic width/precision, explicit argument reordering/reuse, debug hex) decline
//! `fmt_kernel` cleanly and match native Rust output via `Formatter`.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::format;
use core::hint::black_box;

#[no_mangle]
pub fn rust_entry() {
    print_str(&format!("{:*>6}", black_box(42u32)));
    print_str(&format!("{:_<8x}", black_box(0xabu32)));
    print_str(&format!("{:^7}", black_box("hi")));
    let w = black_box(6usize);
    let p = black_box(4usize);
    print_str(&format!("{:0w$x}", black_box(0x1au32), w = w));
    print_str(&format!("{:.p$}", black_box(1.25f64), p = p));
    print_str(&format!("{1} {0} {1:.2}", black_box(10u32), black_box(2.5f64)));
    print_str(&format!("{:#?}|{:x?}|{:X?}", black_box(42u32), black_box(255u32), black_box(255u32)));
    print_str(&format!("{:.3}", black_box("abcdef")));
}
