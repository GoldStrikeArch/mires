//! Constant shift and rotate amounts of 64-bit integers.
//!
//! A shift amount is a `u32`, a JavaScript number, so a 64-bit shift widens it to a BigInt. On
//! `js-ecma-target=2018` a constant amount is widened at compile time and named like any other
//! BigInt constant (`JsCx::to_bigint`), and a constant rotate amount is reduced to `amount & 63`
//! before it is widened (`intrinsics.rs`, `rotate`). The amounts pinned here are the ones whose
//! reduction matters: 0, 63, 64 and past it (`wrapping_shl(70)`, `rotate_right(100)`), on both
//! signednesses, beside the same operations with an amount only known at run time, MurmurHash3's
//! `fmix64` and a 32-bit rotate, which stays a number throughout. The `rotate_*` methods are calls
//! into `core`, which see their amount as a parameter; the `rotate_*` and `unchecked_sh*`
//! intrinsics called here directly are what see it as a literal.
//!
//! The expectations were validated by transplanting this file onto `std` and running it at `-O3`
//! and `-C opt-level=0`, which agree.

#![no_std]
#![no_main]
#![feature(core_intrinsics)]
#![allow(internal_features)]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::intrinsics::{rotate_left, rotate_right, unchecked_shl, unchecked_shr};

#[inline(never)]
fn shifts(x: u64) -> [u64; 6] {
    [
        x >> 33,
        x << 7,
        x.wrapping_shl(70),
        x.wrapping_shr(67),
        x >> 0,
        x << 63,
    ]
}

#[inline(never)]
fn signed_shifts(x: i64) -> [i64; 3] {
    [x >> 5, x << 40, x.wrapping_shr(65)]
}

#[inline(never)]
fn rotates(x: u64) -> [u64; 6] {
    [
        x.rotate_left(31),
        x.rotate_right(27),
        x.rotate_left(0),
        x.rotate_left(64),
        x.rotate_right(100),
        x.rotate_left(63),
    ]
}

#[inline(never)]
fn signed_rotates(x: i64) -> [i64; 2] {
    [x.rotate_left(13), x.rotate_right(63)]
}

#[inline(never)]
fn intrinsic_rotates(x: u64) -> [u64; 5] {
    [
        rotate_left(x, 31),
        rotate_right(x, 27),
        rotate_left(x, 0),
        rotate_left(x, 64),
        rotate_right(x, 100),
    ]
}

#[inline(never)]
fn intrinsic_signed(x: i64) -> [i64; 4] {
    // SAFETY: every amount is below 64.
    unsafe {
        [
            rotate_left(x as u64, 13) as i64,
            rotate_right(x as u64, 63) as i64,
            unchecked_shl(x, 9u32),
            unchecked_shr(x, 62u32),
        ]
    }
}

#[inline(never)]
fn variable(x: u64, by: u32) -> [u64; 3] {
    [x >> (by & 63), x.rotate_left(by), x.wrapping_shl(by)]
}

#[inline(never)]
fn fmix(mut k: u64) -> u64 {
    k ^= k >> 33;
    k = k.wrapping_mul(0xff51_afd7_ed55_8ccd);
    k ^= k >> 33;
    k = k.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    k ^ (k >> 33)
}

#[inline(never)]
fn narrow(x: u32) -> u32 {
    x.rotate_left(5) ^ (x >> 3)
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    for x in [0x8000_0000_0000_0001u64, 0x0123_4567_89AB_CDEF, u64::MAX] {
        for value in shifts(x) {
            print_u64(value);
        }
        for value in rotates(x) {
            print_u64(value);
        }
        for value in intrinsic_rotates(x) {
            print_u64(value);
        }
        print_u64(fmix(x));
    }
    for x in [i64::MIN + 1, -0x0123_4567_89AB, 0x7FFF_0000_FFFF_0000] {
        for value in signed_shifts(x) {
            print_i64(value);
        }
        for value in signed_rotates(x) {
            print_i64(value);
        }
        for value in intrinsic_signed(x) {
            print_i64(value);
        }
    }
    for by in [0u32, 31, 64, 70] {
        for value in variable(0xF0F0_0F0F_FF00_00FF, by) {
            print_u64(value);
        }
    }
    print_u32(narrow(0xDEAD_BEEF));
}
