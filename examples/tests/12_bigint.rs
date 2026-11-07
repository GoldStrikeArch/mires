#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// Wrapping arithmetic has to cross a function boundary: rustc's `arithmetic_overflow`
// lint rejects a literal `i64::MAX + 1` outright, and anything it can const-fold never
// reaches the backend's 64-bit lowering at all.
#[inline(never)]
fn add_i64(a: i64, b: i64) -> i64 {
    a + b
}

#[inline(never)]
fn sub_i64(a: i64, b: i64) -> i64 {
    a - b
}

#[inline(never)]
fn mul_i64(a: i64, b: i64) -> i64 {
    a * b
}

#[inline(never)]
fn div_i64(a: i64, b: i64) -> i64 {
    a / b
}

#[inline(never)]
fn rem_i64(a: i64, b: i64) -> i64 {
    a % b
}

#[inline(never)]
fn add_u64(a: u64, b: u64) -> u64 {
    a + b
}

#[inline(never)]
fn mul_u64(a: u64, b: u64) -> u64 {
    a * b
}

#[inline(never)]
fn div_u64(a: u64, b: u64) -> u64 {
    a / b
}

#[inline(never)]
fn shl_i64(a: i64, n: u32) -> i64 {
    a << n
}

#[inline(never)]
fn shr_i64(a: i64, n: u32) -> i64 {
    a >> n
}

#[inline(never)]
fn shr_u64(a: u64, n: u32) -> u64 {
    a >> n
}

fn classify(x: i64) -> i32 {
    match x {
        0 => 0,
        1 => 1,
        -1 => 2,
        9223372036854775807 => 3,
        -9223372036854775808 => 4,
        4294967296 => 5,
        _ => 9,
    }
}

#[no_mangle]
fn rust_entry() {
    let max: i64 = 9223372036854775807;
    let min: i64 = -9223372036854775808;

    // Wrap at the i64 boundaries.
    print_i64(add_i64(max, 1));
    print_i64(sub_i64(min, 1));
    print_i64(mul_i64(max, 2));
    print_i64(add_i64(min, max));

    // Ordinary 64-bit arithmetic that overflows 32 bits.
    print_i64(mul_i64(1000000000, 1000000000));
    print_i64(add_i64(4294967295, 1));
    print_i64(sub_i64(0, 4294967296));
    print_i64(mul_i64(-3000000000, 3));

    // Truncating division and remainder with negative operands.
    print_i64(div_i64(-7, 2));
    print_i64(rem_i64(-7, 2));
    print_i64(div_i64(7, -2));
    print_i64(rem_i64(7, -2));
    print_i64(div_i64(-7, -2));
    print_i64(rem_i64(-7, -2));
    print_i64(div_i64(-9223372036854775807, 3));

    // u64: arithmetic, wrapping and comparison.
    let umax: u64 = 18446744073709551615;
    print_bool(add_u64(umax, 1) == 0);
    print_bool(umax > 9223372036854775807);
    print_bool(mul_u64(4294967296, 4294967296) == 0);
    print_i64(div_u64(umax, 3) as i64);
    print_i64(mul_u64(4294967296, 4) as i64);
    print_bool(div_u64(umax, 2) == 9223372036854775807);
    print_bool(add_u64(0, 1) < 2);

    // Shifts by a `u32` amount.
    print_i64(shl_i64(1, 40));
    print_i64(shl_i64(1, 62));
    print_i64(shr_i64(max, 32));
    print_i64(shr_i64(-1, 1));
    print_i64(shr_i64(-1024, 4));
    print_i64(shr_u64(umax, 32) as i64);
    print_i64(shr_u64(umax, 1) as i64);

    // Casts in both directions.
    print_i32(max as i32);
    print_i32(min as i32);
    print_i32(1234567890123i64 as i32);
    print_i64(-5i32 as i64);
    print_i64(2147483647i32 as i64);
    print_i64(-1i32 as u32 as i64);
    print_i64(4294967295u32 as i64);
    print_i64(umax as i64);
    print_i64(-1i64 as u64 as i64);
    print_i32(umax as i32);
    print_bool(-1i64 as u64 == 18446744073709551615);

    // Switching on an i64 scrutinee.
    print_i32(classify(0));
    print_i32(classify(1));
    print_i32(classify(-1));
    print_i32(classify(max));
    print_i32(classify(min));
    print_i32(classify(4294967296));
    print_i32(classify(12345));

    print_str("12_bigint ok");
}
