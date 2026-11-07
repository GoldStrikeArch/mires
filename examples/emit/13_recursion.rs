#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// Direct and mutual recursion.

fn factorial(n: i32) -> i32 {
    if n <= 1 { 1 } else { n * factorial(n - 1) }
}

fn is_even(n: i32) -> bool {
    if n == 0 { true } else { is_odd(n - 1) }
}

fn is_odd(n: i32) -> bool {
    if n == 0 { false } else { is_even(n - 1) }
}

#[no_mangle]
fn rust_entry() {
    print_i32(factorial(6));
    print_bool(is_even(10));
    print_bool(is_odd(10));
}
