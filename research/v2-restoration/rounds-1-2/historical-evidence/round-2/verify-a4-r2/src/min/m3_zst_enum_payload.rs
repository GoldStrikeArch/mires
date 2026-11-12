#![no_std]
#![no_main]
#![allow(unused, unreachable_patterns, dead_code)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
#[derive(Clone, Copy)] enum Void {}
#[derive(Clone, Copy)] enum E { Live(char), D(Result<(), Void>), U }
#[inline(never)]
fn cls(e: E) -> u32 { match e { E::Live(p) => p as u32, E::D(..) => 2001, E::U => 1001 } }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    const K: E = E::D(Ok(()));
    static S: E = E::D(Ok(()));
    { let e = K; print_u32(match e { E::Live(p) => p as u32, E::D(..) => 2001, E::U => 1001 }); }
    print_u32(cls(K));
    print_u32(cls(black_box(K)));
    print_u32(cls(S));
    print_u32(cls(E::U));
    print_u32(cls(black_box(E::Live('a'))));
    const O: Option<E> = Some(E::D(Ok(())));
    print_u32(match black_box(O) { Some(e) => cls(e), None => 5 });
}
