#![no_std]
#![no_main]
#![allow(unused, unreachable_patterns, dead_code)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
use core::num::NonZeroU8;
// One inhabited variant plus a dead variant rustc keeps in the layout ([u16; 0] is not a 1-ZST):
// a niche layout with two variants, of which only `Live` can exist.
#[derive(Clone, Copy)] enum Void {}
#[derive(Clone, Copy)] enum E { D0([u16; 0], Void), Live(NonZeroU8) }
#[inline(never)]
fn cls(e: E) -> u32 { match e { E::Live(p) => p.get() as u32, E::D0(..) => 9001 } }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(cls(E::Live(black_box(NonZeroU8::new(8).unwrap()))));
    const K: E = E::Live(NonZeroU8::new(7).unwrap());
    print_u32(cls(K));
    print_u32(cls(black_box(K)));
    const O: Option<E> = Some(E::Live(NonZeroU8::new(6).unwrap()));
    print_u32(match black_box(O) { Some(e) => cls(e), None => 5 });
    const N: Option<E> = None;
    print_u32(match black_box(N) { Some(e) => cls(e), None => 5 });
}
