//! (o) addresses of many short-lived aggregate places taken in one synchronous run. Each is a
//! distinct storage object; natively every one is the same stack slot.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use core::hint::black_box;
use prelude::*;

struct H {
    head: u32,
    tail: u32,
}
#[inline(never)]
fn mk(head: u32) -> H {
    black_box(H { head, tail: head + 1 })
}
#[inline(never)]
fn low_bits(h: &H) -> usize {
    (h as *const H as usize) & 3
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut acc = 0usize;
    let mut sum = 0u32;
    for i in 0..1_200_000u32 {
        let h = mk(i & 7);
        acc |= low_bits(&h);
        sum = sum.wrapping_add(h.head + h.tail);
    }
    print_usize(acc);
    print_u32(sum);
}
