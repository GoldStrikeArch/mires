//! `ref mut rest @ ..` in an array pattern yields a COPY of the tail, so a write through it is
//! lost (the reverse of copy aliasing). Safe Rust, every lane, main and the A1 commit alike.
//! Native (-O0 and -O3): 99 / 99 / 3
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;

#[inline(never)]
fn local() -> u32 {
    let mut c = black_box([1u32, 2, 3]);
    let [_first, ref mut rest @ ..] = c;
    rest[1] = 99;
    c[2]
}

#[inline(never)]
fn through_reference(c: &mut [u32; 3]) -> u32 {
    let [_first, rest @ ..] = c;
    rest[1] = 99;
    c[2]
}

#[inline(never)]
fn read_only(c: &[u32; 3]) -> u32 {
    let [_first, rest @ ..] = c;
    rest[1]
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(local());
    let mut a = black_box([1u32, 2, 3]);
    print_u32(through_reference(&mut a));
    print_u32(read_only(&black_box([1u32, 2, 3])));
}
