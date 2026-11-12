#![no_std]
#![no_main]
#![allow(unused, unreachable_patterns, dead_code)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
#[derive(Clone, Copy)] enum Void {}
#[derive(Clone, Copy)] enum Three { A, B, C }
#[derive(Clone, Copy)] enum E103 { Live(Three), D0(&'static Void, Void) }
#[inline(never)]
fn cls(e: E103) -> u32 { match e { E103::Live(p) => p as u32, E103::D0(..) => 9001 } }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    const K: E103 = E103::Live(Three::B);
    { let e = K; print_u32(match e { E103::Live(p) => p as u32, E103::D0(..) => 9001 }); }
    print_u32(cls(K));
    print_u32(cls(black_box(K)));
    print_u32(cls(E103::Live(black_box(Three::C))));
}
