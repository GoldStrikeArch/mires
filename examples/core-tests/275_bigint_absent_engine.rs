//! A program whose 64-bit constants sit only in functions loads on an engine without `BigInt`.
//!
//! `BigInt` is a runtime requirement of 64-bit code (`CONTRACT.md`), not of a program: before the
//! ES2018 constant hoist (`jsc-link/src/lower/bigint.rs`) a program touched `BigInt` only when a
//! function that computes with a 64-bit value ran, so a program whose 32-bit paths never do loaded
//! and ran on Safari 12.1–13.1 or Firefox 65–67. The hoisted bindings must keep that: they call
//! `BigInt` only when the engine has it, and the first constant an activation reaches is guarded,
//! so it throws the `ReferenceError` the inline `BigInt("…")` threw, at the same point.
//!
//! `scripts/test.sh` also runs this program with the global `BigInt` deleted before the runtime
//! loads (`275_bigint_absent_engine.no_bigint`). Then the 32-bit work prints what it prints with
//! `BigInt`, a 64-bit constant on a branch that is not taken costs nothing (`maybe_wide(.., false)`),
//! the line printed just before the first 64-bit constant still prints, and the program stops with
//! `ReferenceError: BigInt is not defined` there: never a wrong value. `hash64` is exported and
//! holds the program's other 64-bit constants; with `BigInt` it runs last.
//!
//! The expectations were validated by transplanting this file onto `std` and running it at `-O3`
//! and `-C opt-level=0`, which agree.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[inline(never)]
fn narrow(x: u32) -> u32 {
    x.rotate_left(5) ^ (x >> 3)
}

/// A 64-bit constant on a branch the 32-bit path never takes.
#[inline(never)]
fn maybe_wide(x: u32, wide: bool) -> u32 {
    if wide {
        ((x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as u32
    } else {
        x ^ 0x5A5A_5A5A
    }
}

/// The program's other 64-bit constants, in an exported function.
#[unsafe(no_mangle)]
#[inline(never)]
pub fn hash64(x: u32) -> u32 {
    let wide = (x as u64 ^ 0x9E37_79B9_7F4A_7C15).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    (wide >> 40) as u32
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_i32(narrow(0xDEAD_BEEF) as i32);
    print_u32(maybe_wide(7, false));
    print_str("next: the first 64-bit constant");
    print_u32(maybe_wide(7, true));
    print_u32(hash64(3));
}
