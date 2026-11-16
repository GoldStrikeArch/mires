//! 128-bit constants: the widest and the most negative digits a BigInt binding has to spell.
//!
//! On `js-ecma-target=2018` the JIR lane names each distinct BigInt constant by one module-level
//! binding (`jsc-link/src/lower/bigint.rs`). A `u128` or `i128` constant has up to 39 digits and
//! a sign, and must survive exactly: `u128::MAX`, `i128::MIN`, a constant in a static and in a
//! function, and 128-bit shift and rotate amounts past 64. Each value is printed as its two 64-bit
//! halves, high first, through `print_u64`.
//!
//! The expectations were validated by transplanting this file onto `std` and running it at `-O3`
//! and `-C opt-level=0`, which agree.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

const BIG: u128 = 0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210;

static WIDE: [u128; 2] = [BIG, u128::MAX];
static SIGNED_WIDE: [i128; 2] = [i128::MIN, -170_141_183_460_469_231_731_687_303_715_884_105_727];

fn print_u128(x: u128) {
    print_u64((x >> 64) as u64);
    print_u64(x as u64);
}

fn print_i128(x: i128) {
    print_u128(x as u128);
}

#[inline(never)]
fn mul(x: u128) -> u128 {
    x.wrapping_mul(BIG) ^ (x >> 64)
}

#[inline(never)]
fn rot(x: u128) -> u128 {
    x.rotate_left(100) ^ x.rotate_right(3) ^ (x << 127)
}

#[inline(never)]
fn smix(x: i128) -> i128 {
    x.wrapping_mul(-3).wrapping_add(i128::MAX) >> 70
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u128(WIDE[0]);
    print_u128(WIDE[1]);
    print_i128(SIGNED_WIDE[0]);
    print_i128(SIGNED_WIDE[1]);
    print_u128(mul(3));
    print_u128(mul(WIDE[1]));
    print_u128(rot(BIG));
    print_u128(rot(1));
    print_i128(smix(SIGNED_WIDE[1]));
    print_i128(smix(-1));
    print_bool(WIDE[0] == BIG);
    print_bool(SIGNED_WIDE[0] == i128::MIN);
}
