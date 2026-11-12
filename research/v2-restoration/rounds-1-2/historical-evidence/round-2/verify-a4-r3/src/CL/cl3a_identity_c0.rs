//! Closure fn-pointer constants that main called correctly (no arguments, or arguments ignored):
//! calls and pointer equality of two uses of the same constant.
#![no_std]
#![no_main]
#![allow(unpredictable_function_pointer_comparisons)]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

const C0: fn() -> u32 = || 5;
const IGN: fn(u32) -> u32 = |_| 9;
struct Hooks { start: fn() -> u32, stop: fn() -> u32 }
const HOOKS: Hooks = Hooks { start: || 1, stop: || 2 };
static SHOOKS: Hooks = Hooks { start: || 3, stop: || 4 };

#[inline(never)]
fn same0(a: fn() -> u32, b: fn() -> u32) -> bool { black_box(a) == black_box(b) }

#[inline(never)]
fn register(table: &mut [Option<fn() -> u32>; 2], f: fn() -> u32) -> bool {
    // registers a callback once: a duplicate is detected by pointer equality
    for slot in table.iter() {
        if *slot == Some(f) { return false; }
    }
    for slot in table.iter_mut() {
        if slot.is_none() { *slot = Some(f); return true; }
    }
    false
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(C0());
    print_u32(IGN(1));
    print_bool(same0(C0, C0));
    let a = C0; let b = C0;
    print_bool(a == b);
    print_bool(black_box(IGN) == black_box(IGN));
    print_u32((HOOKS.start)() + (HOOKS.stop)());
    print_bool(same0(HOOKS.start, HOOKS.start));
    print_bool(same0(HOOKS.start, HOOKS.stop));
    print_bool(same0(SHOOKS.start, SHOOKS.start));
    let mut table: [Option<fn() -> u32>; 2] = [None, None];
    print_bool(register(&mut table, C0));
    print_bool(register(&mut table, C0));
    print_bool(register(&mut table, HOOKS.stop));
    print_u32(table.iter().flatten().map(|f| f()).sum());
}
