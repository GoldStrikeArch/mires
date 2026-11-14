//! BigInt constants inside closures and inside the arrow functions the lowering prints.
//!
//! On `js-ecma-target=2018` the JIR lane names each distinct BigInt constant by one module-level
//! binding (`jsc-link/src/lower/bigint.rs`). A use inside a closure body — called statically,
//! through `&dyn Fn`, after a `move`, from an iterator adapter — and a use inside a JavaScript
//! arrow the lowering itself prints (`u64::from_le_bytes`'s byte assembly, `to_le_bytes`) all name
//! the same module-level binding, never a local of the enclosing function.
//!
//! The expectations were validated by transplanting this file onto `std` and running it at `-O3`
//! and `-C opt-level=0`, which agree.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[inline(never)]
fn apply<F: Fn(u64) -> u64>(f: F, x: u64) -> u64 {
    f(x)
}

#[inline(never)]
fn apply_dyn(f: &dyn Fn(u64) -> u64, x: u64) -> u64 {
    f(x)
}

#[inline(never)]
fn make_adder(step: u64) -> impl Fn(u64) -> u64 {
    move |x| x.wrapping_add(step).wrapping_add(0x1000_0000_0000_0001)
}

#[inline(never)]
fn from_bytes(bytes: [u8; 8]) -> u64 {
    u64::from_le_bytes(bytes) ^ 0x0F0F_0F0F_0F0F_0F0F
}

#[inline(never)]
fn to_bytes(x: u64) -> [u8; 8] {
    (x ^ 0x0F0F_0F0F_0F0F_0F0F).to_le_bytes()
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let key = 0xDEAD_BEEF_CAFE_BABE_u64;
    let scale = |x: u64| x.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ key;
    print_u64(apply(scale, 3));
    print_u64(apply_dyn(&scale, 5));
    print_u64(apply(|x| x >> 33, u64::MAX));

    let add = make_adder(0x9E37_79B9_7F4A_7C15);
    print_u64(add(1));
    print_u64(apply_dyn(&add, u64::MAX));

    let folded = (0..4u64)
        .map(|i| i.wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
        .fold(0x9E37_79B9_7F4A_7C15u64, |acc, x| acc.rotate_left(31) ^ x);
    print_u64(folded);

    print_u64(from_bytes([1, 2, 3, 4, 5, 6, 7, 0xF8]));
    for byte in to_bytes(0x1122_3344_5566_7788) {
        print_u32(byte as u32);
    }
}
