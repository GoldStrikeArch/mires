//! Moving references to aggregates between buffer elements must not touch the pointees.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
use core::ptr;
use prelude::*;
#[derive(Clone, Copy)]
struct P {
    x: u32,
    y: u32,
}
#[inline(never)]
fn mk(x: u32) -> P {
    black_box(P { x, y: x + 1 })
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let h = mk(1);
    let h2 = mk(2);
    let h3 = mk(3);
    let mut refs: [&P; 2] = black_box([&h, &h2]);
    refs.swap(0, 1);
    print_u32(h.x);
    print_u32(h2.x);
    print_u32(refs[0].x);
    print_bool(ptr::eq(refs[0], &h2));
    let mut v: Vec<&P> = vec![&h, &h2, &h3];
    v.reverse();
    print_u32(h.x + h.y);
    print_u32(h3.x + h3.y);
    print_bool(ptr::eq(v[0], &h3));
    v.sort_by_key(|r| r.x);
    print_u32(v[0].x);
    print_u32(h.x);
}
