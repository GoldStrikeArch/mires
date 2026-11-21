//! Niche-encoded `Option<NonZero<T>>` and `Result` constants across all twelve
//! `NonZero` integer widths (MIRES-006).
//!
//! Under `JSC_RUST_MIR_INLINE=on JSC_RUST_OPT_LEVEL=z`, MIR inlining folds
//! `NonZero::new(5)` into a `ConstValue::Scalar` of type `Option<NonZero<u32>>`;
//! named `const` items have the same scalar representation even without inlining.
//! Every width (`u8`..`u128`, `i8`..`i128`, `usize`, `isize`), `new_unchecked`,
//! `None`, `Result<(), NonZero<T>>` and `Result<NonZero<T>, ()>` must decode to
//! the exact integer payload rather than `{}`.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::num::NonZero;

const OPT_U8_SOME: Option<NonZero<u8>> = NonZero::new(7);
const OPT_U8_MAX: Option<NonZero<u8>> = NonZero::new(u8::MAX);
const OPT_U8_NONE: Option<NonZero<u8>> = None;

const OPT_I8_POS: Option<NonZero<i8>> = NonZero::new(9);
const OPT_I8_NEG: Option<NonZero<i8>> = NonZero::new(-1);
const OPT_I8_MIN: Option<NonZero<i8>> = NonZero::new(i8::MIN);
const OPT_I8_NONE: Option<NonZero<i8>> = None;

const OPT_U16_SOME: Option<NonZero<u16>> = NonZero::new(1025);
const OPT_U16_NONE: Option<NonZero<u16>> = None;

const OPT_I16_NEG: Option<NonZero<i16>> = NonZero::new(-32768);
const OPT_I16_NONE: Option<NonZero<i16>> = None;

const OPT_U32_SOME: Option<NonZero<u32>> = NonZero::new(5);
const OPT_U32_UNCHECKED: Option<NonZero<u32>> = Some(unsafe { NonZero::new_unchecked(42) });
const OPT_U32_MAX: Option<NonZero<u32>> = NonZero::new(u32::MAX);
const OPT_U32_NONE: Option<NonZero<u32>> = None;

const OPT_I32_NEG: Option<NonZero<i32>> = NonZero::new(-77);
const OPT_I32_MIN: Option<NonZero<i32>> = NonZero::new(i32::MIN);
const OPT_I32_NONE: Option<NonZero<i32>> = None;

const OPT_U64_SOME: Option<NonZero<u64>> = NonZero::new(0x0123_4567_89ab_cdef);
const OPT_U64_MAX: Option<NonZero<u64>> = NonZero::new(u64::MAX);
const OPT_U64_NONE: Option<NonZero<u64>> = None;

const OPT_I64_NEG: Option<NonZero<i64>> = NonZero::new(-9_000_000_000_005);
const OPT_I64_MIN: Option<NonZero<i64>> = NonZero::new(i64::MIN);
const OPT_I64_NONE: Option<NonZero<i64>> = None;

const OPT_U128_SOME: Option<NonZero<u128>> =
    NonZero::new(0x0123_4567_89ab_cdef_fedc_ba98_7654_3210);
const OPT_U128_MAX: Option<NonZero<u128>> = NonZero::new(u128::MAX);
const OPT_U128_NONE: Option<NonZero<u128>> = None;

const OPT_I128_NEG: Option<NonZero<i128>> = NonZero::new(-5);
const OPT_I128_MIN: Option<NonZero<i128>> = NonZero::new(i128::MIN);
const OPT_I128_NONE: Option<NonZero<i128>> = None;

const OPT_USIZE_SOME: Option<NonZero<usize>> = NonZero::new(12345);
const OPT_USIZE_NONE: Option<NonZero<usize>> = None;

const OPT_ISIZE_NEG: Option<NonZero<isize>> = NonZero::new(-54321);
const OPT_ISIZE_NONE: Option<NonZero<isize>> = None;

const BARE_U32: NonZero<u32> = unsafe { NonZero::new_unchecked(5) };
const BARE_I128: NonZero<i128> = unsafe { NonZero::new_unchecked(-99) };

const RES_UNIT_ERR: Result<(), NonZero<u32>> = Err(unsafe { NonZero::new_unchecked(13) });
const RES_UNIT_OK: Result<(), NonZero<u32>> = Ok(());
const RES_NZ_OK: Result<NonZero<i64>, ()> = Ok(unsafe { NonZero::new_unchecked(-44) });
const RES_NZ_ERR: Result<NonZero<i64>, ()> = Err(());
const RES_U128_ERR: Result<(), NonZero<u128>> = Err(unsafe { NonZero::new_unchecked(u128::MAX) });
const RES_U128_OK: Result<(), NonZero<u128>> = Ok(());

#[inline(never)]
fn get_u8(v: Option<NonZero<u8>>) -> u32 {
    match v {
        Some(n) => n.get() as u32,
        None => 0,
    }
}

#[inline(never)]
fn get_i8(v: Option<NonZero<i8>>) -> i32 {
    match v {
        Some(n) => n.get() as i32,
        None => 0,
    }
}

#[inline(never)]
fn get_u16(v: Option<NonZero<u16>>) -> u32 {
    match v {
        Some(n) => n.get() as u32,
        None => 0,
    }
}

#[inline(never)]
fn get_i16(v: Option<NonZero<i16>>) -> i32 {
    match v {
        Some(n) => n.get() as i32,
        None => 0,
    }
}

#[inline(never)]
fn get_u32(v: Option<NonZero<u32>>) -> u32 {
    match v {
        Some(n) => n.get(),
        None => 0,
    }
}

#[inline(never)]
fn get_i32(v: Option<NonZero<i32>>) -> i32 {
    match v {
        Some(n) => n.get(),
        None => 0,
    }
}

#[inline(never)]
fn get_u64(v: Option<NonZero<u64>>) -> u64 {
    match v {
        Some(n) => n.get(),
        None => 0,
    }
}

#[inline(never)]
fn get_i64(v: Option<NonZero<i64>>) -> i64 {
    match v {
        Some(n) => n.get(),
        None => 0,
    }
}

#[inline(never)]
fn print_opt_u128(v: Option<NonZero<u128>>) {
    match v {
        Some(n) => {
            let x = n.get();
            print_u64((x >> 64) as u64);
            print_u64(x as u64);
        }
        None => print_i32(-1),
    }
}

#[inline(never)]
fn print_opt_i128(v: Option<NonZero<i128>>) {
    match v {
        Some(n) => {
            let x = n.get();
            print_i64((x >> 64) as i64);
            print_u64(x as u64);
        }
        None => print_i32(-1),
    }
}

#[inline(never)]
fn get_usize(v: Option<NonZero<usize>>) -> usize {
    match v {
        Some(n) => n.get(),
        None => 0,
    }
}

#[inline(never)]
fn get_isize(v: Option<NonZero<isize>>) -> i32 {
    match v {
        Some(n) => n.get() as i32,
        None => 0,
    }
}

#[inline(never)]
fn get_bare_u32(v: NonZero<u32>) -> u32 {
    v.get()
}

#[inline(never)]
fn get_bare_i128_low(v: NonZero<i128>) -> i64 {
    v.get() as i64
}

#[inline(never)]
fn get_res_unit_u32(v: Result<(), NonZero<u32>>) -> i32 {
    match v {
        Ok(()) => 0,
        Err(n) => n.get() as i32,
    }
}

#[inline(never)]
fn get_res_i64_unit(v: Result<NonZero<i64>, ()>) -> i64 {
    match v {
        Ok(n) => n.get(),
        Err(()) => 0,
    }
}

#[inline(never)]
fn print_res_u128(v: Result<(), NonZero<u128>>) {
    match v {
        Ok(()) => print_i32(0),
        Err(n) => {
            let x = n.get();
            print_u64((x >> 64) as u64);
            print_u64(x as u64);
        }
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Original MIRES-006 repro: MIR inlining folds `NonZero::new(5)` into a constant.
    print_str("inline-fold");
    print_u32(get_u32(NonZero::new(5)));
    print_u32(get_u32(NonZero::new(0)));
    print_u32(get_u32(None));
    print_i32(get_i32(NonZero::new(-5)));
    print_i64(get_i64(NonZero::new(-1234567890123)));

    print_str("u8/i8");
    print_u32(get_u8(OPT_U8_SOME));
    print_u32(get_u8(OPT_U8_MAX));
    print_u32(get_u8(OPT_U8_NONE));
    print_i32(get_i8(OPT_I8_POS));
    print_i32(get_i8(OPT_I8_NEG));
    print_i32(get_i8(OPT_I8_MIN));
    print_i32(get_i8(OPT_I8_NONE));

    print_str("u16/i16");
    print_u32(get_u16(OPT_U16_SOME));
    print_u32(get_u16(OPT_U16_NONE));
    print_i32(get_i16(OPT_I16_NEG));
    print_i32(get_i16(OPT_I16_NONE));

    print_str("u32/i32");
    print_u32(get_u32(OPT_U32_SOME));
    print_u32(get_u32(OPT_U32_UNCHECKED));
    print_u32(get_u32(OPT_U32_MAX));
    print_u32(get_u32(OPT_U32_NONE));
    print_i32(get_i32(OPT_I32_NEG));
    print_i32(get_i32(OPT_I32_MIN));
    print_i32(get_i32(OPT_I32_NONE));

    print_str("u64/i64");
    print_u64(get_u64(OPT_U64_SOME));
    print_u64(get_u64(OPT_U64_MAX));
    print_u64(get_u64(OPT_U64_NONE));
    print_i64(get_i64(OPT_I64_NEG));
    print_i64(get_i64(OPT_I64_MIN));
    print_i64(get_i64(OPT_I64_NONE));

    print_str("u128/i128");
    print_opt_u128(OPT_U128_SOME);
    print_opt_u128(OPT_U128_MAX);
    print_opt_u128(OPT_U128_NONE);
    print_opt_i128(OPT_I128_NEG);
    print_opt_i128(OPT_I128_MIN);
    print_opt_i128(OPT_I128_NONE);

    print_str("usize/isize/bare");
    print_usize(get_usize(OPT_USIZE_SOME));
    print_usize(get_usize(OPT_USIZE_NONE));
    print_i32(get_isize(OPT_ISIZE_NEG));
    print_i32(get_isize(OPT_ISIZE_NONE));
    print_u32(get_bare_u32(BARE_U32));
    print_i64(get_bare_i128_low(BARE_I128));

    print_str("result");
    print_i32(get_res_unit_u32(RES_UNIT_ERR));
    print_i32(get_res_unit_u32(RES_UNIT_OK));
    print_i64(get_res_i64_unit(RES_NZ_OK));
    print_i64(get_res_i64_unit(RES_NZ_ERR));
    print_res_u128(RES_U128_ERR);
    print_res_u128(RES_U128_OK);

    print_str("eq");
    print_bool(OPT_U32_SOME == black_box(NonZero::new(5)));
    print_bool(OPT_U32_SOME == black_box(NonZero::new(6)));
    print_bool(OPT_U32_NONE == black_box(None));
    print_bool(OPT_I8_MIN == black_box(NonZero::new(i8::MIN)));
    print_bool(OPT_I128_MIN == black_box(NonZero::new(i128::MIN)));
    print_bool(OPT_U128_MAX == black_box(NonZero::new(u128::MAX)));
}
