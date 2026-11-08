//! Physical slice lanes with canonical exported and function-pointer boundaries.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

/// The exported symbol keeps the canonical `{ buf, off, len }` argument. Direct Rust callers use
/// one private physical version whose signature is `(buf, off, len)`.
#[inline(never)]
#[unsafe(no_mangle)]
pub fn slice_fold(s: &[u8]) -> u32 {
    let mut acc = 0x811c_9dc5u32;
    let mut i = 0usize;
    let n = s.len();
    while i + 4 <= n {
        let word = (s[i] as u32)
            | ((s[i + 1] as u32) << 8)
            | ((s[i + 2] as u32) << 16)
            | ((s[i + 3] as u32) << 24);
        acc = (acc ^ word).wrapping_mul(0x0100_0193);
        i += 4;
    }
    while i < n {
        acc = (acc ^ s[i] as u32).wrapping_mul(0x0100_0193);
        i += 1;
    }
    acc
}

#[inline(never)]
fn direct(s: &[u8]) -> u32 {
    slice_fold(s)
}

#[inline(never)]
fn indirect(s: &[u8], callback: fn(&[u8]) -> u32) -> u32 {
    callback(s)
}

#[inline(never)]
fn fixed_subslice(s: &[u8]) -> u32 {
    let middle = &s[2..6];
    middle[0] as u32 + middle[3] as u32
}

#[inline(never)]
fn aligned_prefix(s: &[u8]) -> u32 {
    let width = s.len() - s.len() % 4;
    let prefix = &s[..width];
    let mut total = 0u32;
    let mut i = 0;
    while i < prefix.len() {
        total = total.wrapping_add(prefix[i] as u32);
        i += 1;
    }
    total
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let data = [
        0x01u8, 0x7f, 0x80, 0xff, 0x10, 0x20, 0x30, 0x40, 0xaa, 0xbb, 0xcc,
    ];
    print_u32(direct(&data));
    print_u32(indirect(&data, slice_fold));
    print_u32(direct(&data[..8]));
    print_u32(indirect(&[], slice_fold));
    print_u32(fixed_subslice(&data));
    print_u32(aligned_prefix(&data));
}
