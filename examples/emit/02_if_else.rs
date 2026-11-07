#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// An if / else-if / else chain, the shape the structurizer turns into nested ifs.

fn classify(n: i32) -> i32 {
    if n < 0 {
        -1
    } else if n == 0 {
        0
    } else if n < 10 {
        1
    } else {
        2
    }
}

#[no_mangle]
fn rust_entry() {
    print_i32(classify(-3));
    print_i32(classify(0));
    print_i32(classify(4));
    print_i32(classify(40));
}
