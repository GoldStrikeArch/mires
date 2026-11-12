//! The same miscompile without the padding: wrong with BOTH linkers, because the base linker
//! still has ledger room for the `set` site in a program this small.

#![no_std]
#![no_main]
#![allow(warnings)]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

#[derive(Clone, Copy)]
struct S {
    f: u8,
}

fn set(s: &mut S, v: u8) {
    s.f = v;
}

fn part() {
    let mut a = S { f: black_box(1) };
    if black_box(true) {
        a = S { f: black_box(7) };
    }
    set(&mut a, black_box(254));
    print_i32(a.f as i32);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    part();
}
