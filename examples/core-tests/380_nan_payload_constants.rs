//! `f32`/`f64` NaN-payload constants must preserve their payload and sign bit
//! across both lanes, unwind/abort, and MIR inlining (MIRES-023), while canonical
//! quiet NaNs and all non-NaN edge constants continue to round-trip accurately.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

const F32_CANON_BITS: u32 = 0x7fc0_0000;
const F32_NEG_CANON_BITS: u32 = 0xffc0_0000;
const F32_QNAN1_BITS: u32 = 0x7fc0_0001;
const F32_QNAN_MID_BITS: u32 = 0x7fc1_2345;
const F32_QNAN_ALL1_BITS: u32 = 0x7fff_ffff;
const F32_NEG_QNAN1_BITS: u32 = 0xffc0_0001;
const F32_NEG_QNAN_ALL1_BITS: u32 = 0xffff_ffff;

const F32_CANON: f32 = f32::NAN;
const F32_NEG_CANON: f32 = -f32::NAN;
const F32_QNAN1: f32 = f32::from_bits(F32_QNAN1_BITS);
const F32_QNAN_MID: f32 = f32::from_bits(F32_QNAN_MID_BITS);
const F32_QNAN_ALL1: f32 = f32::from_bits(F32_QNAN_ALL1_BITS);
const F32_NEG_QNAN1: f32 = f32::from_bits(F32_NEG_QNAN1_BITS);
const F32_NEG_QNAN_ALL1: f32 = f32::from_bits(F32_NEG_QNAN_ALL1_BITS);

const F64_CANON_BITS: u64 = 0x7ff8_0000_0000_0000;
const F64_NEG_CANON_BITS: u64 = 0xfff8_0000_0000_0000;
const F64_QNAN1_BITS: u64 = 0x7ff8_0000_0000_0001;
const F64_QNAN_MID_BITS: u64 = 0x7ff8_1234_5678_9abc;
const F64_QNAN_ALL1_BITS: u64 = 0x7fff_ffff_ffff_ffff;
const F64_NEG_QNAN1_BITS: u64 = 0xfff8_0000_0000_0001;
const F64_NEG_QNAN_ALL1_BITS: u64 = 0xffff_ffff_ffff_ffff;

const F64_CANON: f64 = f64::NAN;
const F64_NEG_CANON: f64 = -f64::NAN;
const F64_QNAN1: f64 = f64::from_bits(F64_QNAN1_BITS);
const F64_QNAN_MID: f64 = f64::from_bits(F64_QNAN_MID_BITS);
const F64_QNAN_ALL1: f64 = f64::from_bits(F64_QNAN_ALL1_BITS);
const F64_NEG_QNAN1: f64 = f64::from_bits(F64_NEG_QNAN1_BITS);
const F64_NEG_QNAN_ALL1: f64 = f64::from_bits(F64_NEG_QNAN_ALL1_BITS);

const F32_POS_ZERO: f32 = 0.0;
const F32_NEG_ZERO: f32 = -0.0;
const F32_SUBNORMAL: f32 = f32::from_bits(1);
const F64_POS_ZERO: f64 = 0.0;
const F64_NEG_ZERO: f64 = -0.0;
const F64_SUBNORMAL: f64 = f64::from_bits(1);

#[inline(never)]
fn bits32(x: f32) -> u32 {
    x.to_bits()
}

#[inline(never)]
fn bits64(x: f64) -> u64 {
    x.to_bits()
}

#[inline(never)]
fn check_f32_nan(x: f32, expected: u32, expect_neg: bool) {
    print_bool(x.is_nan());
    print_bool(x.is_sign_negative() == expect_neg);
    print_bool(x.is_sign_positive() != expect_neg);
    let b = bits32(x);
    print_u32(b);
    print_bool(b == expected);
}

#[inline(never)]
fn check_f64_nan(x: f64, expected: u64, expect_neg: bool) {
    print_bool(x.is_nan());
    print_bool(x.is_sign_negative() == expect_neg);
    print_bool(x.is_sign_positive() != expect_neg);
    let b = bits64(x);
    print_u64(b);
    print_bool(b == expected);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. f32 NaN constants (canonical, positive payload, negative canonical, negative payload).
    check_f32_nan(F32_CANON, F32_CANON_BITS, false);
    check_f32_nan(F32_QNAN1, F32_QNAN1_BITS, false);
    check_f32_nan(F32_QNAN_MID, F32_QNAN_MID_BITS, false);
    check_f32_nan(F32_QNAN_ALL1, F32_QNAN_ALL1_BITS, false);
    check_f32_nan(F32_NEG_CANON, F32_NEG_CANON_BITS, true);
    check_f32_nan(F32_NEG_QNAN1, F32_NEG_QNAN1_BITS, true);
    check_f32_nan(F32_NEG_QNAN_ALL1, F32_NEG_QNAN_ALL1_BITS, true);

    // Runtime-written from_bits (folds to a constant under JSC_RUST_MIR_INLINE=on).
    check_f32_nan(f32::from_bits(F32_QNAN1_BITS), F32_QNAN1_BITS, false);
    check_f32_nan(f32::from_bits(F32_NEG_QNAN1_BITS), F32_NEG_QNAN1_BITS, true);

    // 2. f64 NaN constants (canonical, positive payload, negative canonical, negative payload).
    check_f64_nan(F64_CANON, F64_CANON_BITS, false);
    check_f64_nan(F64_QNAN1, F64_QNAN1_BITS, false);
    check_f64_nan(F64_QNAN_MID, F64_QNAN_MID_BITS, false);
    check_f64_nan(F64_QNAN_ALL1, F64_QNAN_ALL1_BITS, false);
    check_f64_nan(F64_NEG_CANON, F64_NEG_CANON_BITS, true);
    check_f64_nan(F64_NEG_QNAN1, F64_NEG_QNAN1_BITS, true);
    check_f64_nan(F64_NEG_QNAN_ALL1, F64_NEG_QNAN_ALL1_BITS, true);

    // Runtime-written from_bits (folds to a constant under JSC_RUST_MIR_INLINE=on).
    check_f64_nan(f64::from_bits(F64_QNAN1_BITS), F64_QNAN1_BITS, false);
    check_f64_nan(f64::from_bits(F64_NEG_QNAN1_BITS), F64_NEG_QNAN1_BITS, true);

    // 3. total_cmp observes NaN sign and payload ordering in safe Rust.
    print_ordering(F32_NEG_QNAN_ALL1.total_cmp(&F32_NEG_QNAN1));
    print_ordering(F32_NEG_QNAN1.total_cmp(&F32_NEG_CANON));
    print_ordering(F32_NEG_CANON.total_cmp(&F32_CANON));
    print_ordering(F32_CANON.total_cmp(&F32_QNAN1));
    print_ordering(F32_QNAN1.total_cmp(&F32_QNAN_MID));
    print_ordering(F32_QNAN_MID.total_cmp(&F32_QNAN_ALL1));

    print_ordering(F64_NEG_QNAN_ALL1.total_cmp(&F64_NEG_QNAN1));
    print_ordering(F64_NEG_QNAN1.total_cmp(&F64_NEG_CANON));
    print_ordering(F64_NEG_CANON.total_cmp(&F64_CANON));
    print_ordering(F64_CANON.total_cmp(&F64_QNAN1));
    print_ordering(F64_QNAN1.total_cmp(&F64_QNAN_MID));
    print_ordering(F64_QNAN_MID.total_cmp(&F64_QNAN_ALL1));

    // 4. Non-NaN edge constants stay bit-exact.
    print_u32(bits32(F32_POS_ZERO));
    print_u32(bits32(F32_NEG_ZERO));
    print_u32(bits32(f32::INFINITY));
    print_u32(bits32(f32::NEG_INFINITY));
    print_u32(bits32(F32_SUBNORMAL));
    print_u32(bits32(f32::MIN_POSITIVE));
    print_u32(bits32(f32::MAX));
    print_u32(bits32(f32::MIN));

    print_u64(bits64(F64_POS_ZERO));
    print_u64(bits64(F64_NEG_ZERO));
    print_u64(bits64(f64::INFINITY));
    print_u64(bits64(f64::NEG_INFINITY));
    print_u64(bits64(F64_SUBNORMAL));
    print_u64(bits64(f64::MIN_POSITIVE));
    print_u64(bits64(f64::MAX));
    print_u64(bits64(f64::MIN));

    // 5. Repeated to_bits calls across >=20 iterations must not canonicalize NaN payloads.
    let mut ok32 = true;
    let mut ok64 = true;
    let mut i = 0u32;
    while i < 25 {
        if bits32(F32_QNAN1) != F32_QNAN1_BITS || bits32(F32_NEG_QNAN1) != F32_NEG_QNAN1_BITS {
            ok32 = false;
        }
        if bits64(F64_QNAN1) != F64_QNAN1_BITS || bits64(F64_NEG_QNAN1) != F64_NEG_QNAN1_BITS {
            ok64 = false;
        }
        i += 1;
    }
    print_bool(ok32);
    print_bool(ok64);
}
