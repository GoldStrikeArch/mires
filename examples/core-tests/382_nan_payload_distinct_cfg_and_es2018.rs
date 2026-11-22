//! Distinct NaN-payload constants across branches, match arms, loop phis,
//! f32<->f64 widening/narrowing, and BigInt-free ES2018 execution (MIRES-023).

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

const F32_A: f32 = f32::from_bits(0x7fc0_0001);
const F32_B: f32 = f32::from_bits(0x7fc0_0002);
const F32_NEG: f32 = -f32::NAN;
const F32_CANON: f32 = f32::NAN;

const F64_A: f64 = f64::from_bits(0x7ff8_0000_0000_0001);
const F64_B: f64 = f64::from_bits(0x7ff8_0000_0000_0002);
const F64_NEG: f64 = -f64::NAN;
const F64_CANON: f64 = f64::NAN;

#[inline(never)]
fn select_f32(k: u32) -> f32 {
    match k {
        0 => F32_A,
        1 => F32_B,
        2 => F32_NEG,
        _ => F32_CANON,
    }
}

#[inline(never)]
fn select_f64(k: u32) -> f64 {
    match k {
        0 => F64_A,
        1 => F64_B,
        2 => F64_NEG,
        _ => F64_CANON,
    }
}

#[inline(never)]
fn branch_f32(c: bool) -> f32 {
    if c {
        F32_A
    } else {
        F32_B
    }
}

#[inline(never)]
fn branch_f64(c: bool) -> f64 {
    if c {
        F64_A
    } else {
        F64_B
    }
}

#[inline(never)]
fn loop_phi_f64(n: u32) -> f64 {
    let mut acc = F64_A;
    let mut i = 0u32;
    while i < n {
        acc = if i % 2 == 0 { F64_B } else { F64_NEG };
        i += 1;
    }
    acc
}

#[inline(never)]
fn print_f64_halves(x: f64) {
    let bits = x.to_bits();
    let lo = (bits & 0xffff_ffff) as u32;
    let hi = (bits >> 32) as u32;
    print_u32(lo);
    print_u32(hi);
}

#[inline(never)]
fn widen_narrow_f32(x: f32) -> u32 {
    let wide: f64 = x as f64;
    let back: f32 = black_box(wide) as f32;
    back.to_bits()
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. Multi-arm match returning distinct NaNs (must not merge).
    print_u32(select_f32(black_box(0)).to_bits());
    print_u32(select_f32(black_box(1)).to_bits());
    print_u32(select_f32(black_box(2)).to_bits());
    print_u32(select_f32(black_box(3)).to_bits());

    print_f64_halves(select_f64(black_box(0)));
    print_f64_halves(select_f64(black_box(1)));
    print_f64_halves(select_f64(black_box(2)));
    print_f64_halves(select_f64(black_box(3)));

    // 2. Two-arm branch returning distinct NaNs.
    print_u32(branch_f32(black_box(true)).to_bits());
    print_u32(branch_f32(black_box(false)).to_bits());
    print_f64_halves(branch_f64(black_box(true)));
    print_f64_halves(branch_f64(black_box(false)));

    // 3. Loop-carried phi over distinct NaN constants.
    print_f64_halves(loop_phi_f64(black_box(0)));
    print_f64_halves(loop_phi_f64(black_box(1)));
    print_f64_halves(loop_phi_f64(black_box(2)));

    // 4. f32 -> f64 -> f32 quiet NaN payload round-trip.
    print_u32(widen_narrow_f32(F32_A));
    print_u32(widen_narrow_f32(F32_B));
    print_u32(widen_narrow_f32(F32_NEG));
}
