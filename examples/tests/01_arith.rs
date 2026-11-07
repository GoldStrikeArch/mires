#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

#[no_mangle]
fn rust_entry() {
    // i32 arithmetic
    let a: i32 = 17;
    let b: i32 = 5;
    print_i32(a + b);
    print_i32(a - b);
    print_i32(a * b);
    print_i32(a / b);
    print_i32(a % b);
    print_i32(-a);

    // i32 bitwise and shifts
    print_i32(a & b);
    print_i32(a | b);
    print_i32(a ^ b);
    print_i32(a << 3);
    print_i32(a >> 2);
    print_i32(!a);
    print_i32((-a) >> 1);

    // compound assignment
    let mut c: i32 = 1;
    c += 9;
    c *= 3;
    c -= 5;
    c /= 5;
    c %= 3;
    print_i32(c);

    // narrower integer types, printed through i32
    let u: u8 = 200;
    let v: u8 = 55;
    print_i32((u + v) as i32);
    print_i32((u - v) as i32);
    print_i32((u / v) as i32);
    let s: i8 = -100;
    print_i32(s as i32);
    print_i32((s / 4) as i32);
    let w: i16 = -3000;
    print_i32((w + 1000) as i32);
    let x16: u16 = 60000;
    print_i32((x16 / 3) as i32);
    let big: u32 = 4000000000;
    print_i32((big / 1000) as i32);
    let n: usize = 12;
    print_i32((n * 3) as i32);
    let m: isize = -7;
    print_i32((m * 6) as i32);

    // f64 arithmetic
    let p: f64 = 2.5;
    let q: f64 = 4.0;
    print_f64(p + q);
    print_f64(q - p);
    print_f64(p * q);
    print_f64(q / p);
    print_f64(q % p);
    print_f64(-p);
    print_f64(q);
    print_f64(7.0 / 2.0);

    // casts between int and float
    print_f64(a as f64);
    print_i32(p as i32);

    // bool logic
    let t: bool = true;
    let f: bool = false;
    print_bool(t & f);
    print_bool(t | f);
    print_bool(t ^ f);
    print_bool(!t);
    print_bool(t && f);
    print_bool(t || f);

    // comparisons
    print_bool(a == 17);
    print_bool(a != b);
    print_bool(a < b);
    print_bool(a <= 17);
    print_bool(a > b);
    print_bool(a >= 18);
    print_bool(p < q);
    print_bool(p == 2.5);
    let ch: char = 'q';
    print_bool(ch == 'q');
    print_bool(ch < 'z');

    print_str("01_arith ok");
}
