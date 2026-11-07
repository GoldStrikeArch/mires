#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

fn factorial(n: i32) -> i32 {
    if n <= 1 {
        1
    } else {
        n * factorial(n - 1)
    }
}

fn fib(n: i32) -> i32 {
    if n < 2 {
        n
    } else {
        fib(n - 1) + fib(n - 2)
    }
}

// mutual recursion
fn is_even(n: i32) -> bool {
    if n == 0 {
        true
    } else {
        is_odd(n - 1)
    }
}

fn is_odd(n: i32) -> bool {
    if n == 0 {
        false
    } else {
        is_even(n - 1)
    }
}

// tail-style recursion
fn gcd(a: i32, b: i32) -> i32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn sum_to(n: i32) -> i32 {
    if n == 0 { 0 } else { n + sum_to(n - 1) }
}

fn pow(base: i32, exp: i32) -> i32 {
    if exp == 0 { 1 } else { base * pow(base, exp - 1) }
}

// mutual recursion with accumulators
fn down(n: i32, acc: i32) -> i32 {
    if n == 0 { acc } else { up(n - 1, acc + n) }
}

fn up(n: i32, acc: i32) -> i32 {
    if n == 0 { acc } else { down(n - 1, acc * 2) }
}

#[no_mangle]
fn rust_entry() {
    print_i32(factorial(0));
    print_i32(factorial(1));
    print_i32(factorial(5));
    print_i32(factorial(10));

    print_i32(fib(0));
    print_i32(fib(1));
    print_i32(fib(10));
    print_i32(fib(20));

    print_bool(is_even(0));
    print_bool(is_even(7));
    print_bool(is_odd(7));
    print_bool(is_even(10));
    print_bool(is_odd(10));

    print_i32(gcd(48, 18));
    print_i32(gcd(17, 5));
    print_i32(gcd(100, 75));

    print_i32(sum_to(100));

    print_i32(pow(2, 10));
    print_i32(pow(3, 5));

    print_i32(down(5, 0));

    print_str("05_recursion ok");
}
