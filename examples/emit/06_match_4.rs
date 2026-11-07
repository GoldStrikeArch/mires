#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// A four arm match, including an or-pattern and a range: still one flat switch.

fn small(n: i32) -> i32 {
    match n {
        0 => 100,
        1 | 2 => 200,
        3..=5 => 300,
        _ => 400,
    }
}

#[no_mangle]
fn rust_entry() {
    print_i32(small(0));
    print_i32(small(2));
    print_i32(small(4));
    print_i32(small(9));
}
