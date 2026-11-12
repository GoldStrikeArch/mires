#![no_std]
#![no_main]
#![allow(unused, unreachable_patterns, dead_code)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
#[derive(Clone, Copy)] enum Void {}
#[derive(Clone, Copy)] enum E4 { Live(u64), D0(u8, Void) }
#[inline(never)]
fn cls(e: E4) -> u32 { match e { E4::Live(p) => p as u32, E4::D0(..) => 9001 } }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    const K: E4 = E4::Live(7);
    { let e = K; print_u32(match e { E4::Live(p) => p as u32, E4::D0(..) => 9001 }); }
    print_u32(cls(K));
    print_u32(cls(black_box(K)));
    print_u32(cls(E4::Live(black_box(8))));
}
