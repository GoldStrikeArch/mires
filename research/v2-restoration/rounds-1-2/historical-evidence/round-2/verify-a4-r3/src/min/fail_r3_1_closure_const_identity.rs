//! FAIL (round 3): two uses of one closure `fn`-pointer constant no longer compare equal.
//! Native (-O0, -O3): 5 true true 1. Main (ea5604f), all four modes: 5 true true 1.
//! e2245df, all four modes: 5 false false 2 (each evaluation of the constant builds a fresh
//! `($a0..) => shim(undefined, $a0..)` wrapper, so a callback registered twice is not recognized).
#![no_std]
#![no_main]
#![allow(unpredictable_function_pointer_comparisons)]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

const C0: fn() -> u32 = || 5;

#[inline(never)]
fn register(table: &mut [Option<fn() -> u32>; 2], f: fn() -> u32) {
    if table.iter().any(|slot| *slot == Some(f)) {
        return;
    }
    if let Some(slot) = table.iter_mut().find(|slot| slot.is_none()) {
        *slot = Some(f);
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(C0());
    print_bool(black_box(C0) == black_box(C0));
    let a = C0;
    let b = C0;
    print_bool(a == b);
    let mut table = [None, None];
    register(&mut table, C0);
    register(&mut table, C0);
    print_u32(table.iter().flatten().count() as u32);
}
