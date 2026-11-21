//! Promoted references, `const`/`static` aggregates, nested `Option<Option<NonZero<T>>>`,
//! and closures capturing `Option<NonZero<T>>` constants (MIRES-006).
//!
//! Exercises `ConstValue::Indirect`, `static` initializers, promoted `&Option<NonZero<T>>`,
//! and `ConstValue::Scalar` closures capturing `Option<NonZero<T>>` alongside runtime values.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::num::NonZero;

#[derive(Copy, Clone, PartialEq)]
struct Config {
    limit: Option<NonZero<u32>>,
    offset: Option<NonZero<i16>>,
    wide: Option<NonZero<i128>>,
}

const NZ5: NonZero<u32> = unsafe { NonZero::new_unchecked(5) };
const NZ11: NonZero<u32> = unsafe { NonZero::new_unchecked(11) };

const PROMOTED_SOME: &Option<NonZero<u32>> = &NonZero::new(5);
const PROMOTED_NONE: &Option<NonZero<u32>> = &None;
const PROMOTED_I8_NEG: &Option<NonZero<i8>> = &NonZero::new(-42);
const PROMOTED_I128_NEG: &Option<NonZero<i128>> = &NonZero::new(-123456);

const NESTED_U32_NONE: Option<Option<NonZero<u32>>> = None;
const NESTED_U32_SOME_NONE: Option<Option<NonZero<u32>>> = Some(None);
const NESTED_U32_SOME_SOME: Option<Option<NonZero<u32>>> = Some(NonZero::new(77));

const NESTED_I128_NONE: Option<Option<NonZero<i128>>> = None;
const NESTED_I128_SOME_NONE: Option<Option<NonZero<i128>>> = Some(None);
const NESTED_I128_SOME_SOME: Option<Option<NonZero<i128>>> = Some(NonZero::new(-999));

const CONST_PAIR: (u32, Option<NonZero<u32>>) = (3, NonZero::new(8));
const CONST_ARR: [Option<NonZero<u32>>; 3] = [NonZero::new(1), None, NonZero::new(3)];
const CONST_CFG: Config = Config {
    limit: NonZero::new(64),
    offset: NonZero::new(-12),
    wide: NonZero::new(-700),
};

static STATIC_OPT: Option<NonZero<u32>> = NonZero::new(55);
static STATIC_NONE: Option<NonZero<u32>> = None;
static STATIC_TABLE: [Option<NonZero<u64>>; 3] =
    [NonZero::new(100), None, NonZero::new(0x0001_0000_0000_0007)];
static STATIC_CFG: Config = Config {
    limit: NonZero::new(128),
    offset: None,
    wide: NonZero::new(i128::MIN),
};

#[inline(never)]
fn read_ref_u32(v: &Option<NonZero<u32>>) -> u32 {
    match *v {
        Some(n) => n.get(),
        None => 0,
    }
}

#[inline(never)]
fn read_ref_i8(v: &Option<NonZero<i8>>) -> i32 {
    match *v {
        Some(n) => n.get() as i32,
        None => 0,
    }
}

#[inline(never)]
fn read_ref_i128_low(v: &Option<NonZero<i128>>) -> i64 {
    match *v {
        Some(n) => n.get() as i64,
        None => 0,
    }
}

#[inline(never)]
fn read_nested_u32(v: Option<Option<NonZero<u32>>>) -> i32 {
    match v {
        None => -2,
        Some(None) => -1,
        Some(Some(n)) => n.get() as i32,
    }
}

#[inline(never)]
fn read_nested_i128(v: Option<Option<NonZero<i128>>>) -> i64 {
    match v {
        None => -2,
        Some(None) => -1,
        Some(Some(n)) => n.get() as i64,
    }
}

#[inline(never)]
fn read_pair(p: (u32, Option<NonZero<u32>>)) -> u32 {
    p.0 * 100 + p.1.map_or(0, NonZero::get)
}

#[inline(never)]
fn read_cfg(c: &Config) -> i64 {
    let a = c.limit.map_or(0, NonZero::get) as i64;
    let b = c.offset.map_or(0, NonZero::get) as i64;
    let w = c
        .wide
        .map_or(0, |n| (n.get() >> 64) as i64 ^ (n.get() as i64));
    a * 1000 + b + w
}

#[inline(never)]
fn run_closure<F: FnOnce(u32) -> u32>(f: F, x: u32) -> u32 {
    f(x)
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_str("promoted");
    print_u32(read_ref_u32(PROMOTED_SOME));
    print_u32(read_ref_u32(PROMOTED_NONE));
    print_i32(read_ref_i8(PROMOTED_I8_NEG));
    print_i64(read_ref_i128_low(PROMOTED_I128_NEG));

    print_str("nested");
    print_i32(read_nested_u32(NESTED_U32_NONE));
    print_i32(read_nested_u32(NESTED_U32_SOME_NONE));
    print_i32(read_nested_u32(NESTED_U32_SOME_SOME));
    print_i64(read_nested_i128(NESTED_I128_NONE));
    print_i64(read_nested_i128(NESTED_I128_SOME_NONE));
    print_i64(read_nested_i128(NESTED_I128_SOME_SOME));

    print_str("const-aggregates");
    print_u32(read_pair(CONST_PAIR));
    print_u32(CONST_ARR[black_box(0)].map_or(0, NonZero::get));
    print_u32(CONST_ARR[black_box(1)].map_or(0, NonZero::get));
    print_u32(CONST_ARR[black_box(2)].map_or(0, NonZero::get));
    print_i64(read_cfg(&CONST_CFG));

    print_str("statics");
    print_u32(read_ref_u32(&STATIC_OPT));
    print_u32(read_ref_u32(&STATIC_NONE));
    print_u64(STATIC_TABLE[black_box(0)].map_or(0, NonZero::get));
    print_u64(STATIC_TABLE[black_box(1)].map_or(0, NonZero::get));
    print_u64(STATIC_TABLE[black_box(2)].map_or(0, NonZero::get));
    print_i64(read_cfg(&STATIC_CFG));

    // Closures capturing constant `NonZero<u32>` and `Option<NonZero<u32>>`:
    // the closure environment itself is a one-scalar constant in MIR.
    print_str("closures");
    const CAP_OPT: Option<NonZero<u32>> = Some(NZ5);
    const CAP_NZ: NonZero<u32> = NZ11;
    let c1 = move |x: u32| match CAP_OPT {
        Some(n) => n.get() + x,
        None => 0,
    };
    let c2 = move |x: u32| CAP_NZ.get() + x;
    let captured_opt = CAP_OPT;
    let c3 = move |x: u32| match captured_opt {
        Some(n) => n.get() + x,
        None => 0,
    };
    let captured_nz = CAP_NZ;
    let c4 = move |x: u32| captured_nz.get() + x;
    print_u32(run_closure(c1, black_box(0)));
    print_u32(run_closure(c2, black_box(0)));
    print_u32(run_closure(c3, black_box(1)));
    print_u32(run_closure(c4, black_box(1)));

    print_str("eq");
    print_bool(*PROMOTED_SOME == black_box(NonZero::new(5)));
    print_bool(STATIC_OPT == black_box(NonZero::new(55)));
    print_bool(NESTED_U32_SOME_SOME == black_box(Some(NonZero::new(77))));
    print_bool(CONST_CFG == black_box(CONST_CFG));
}
