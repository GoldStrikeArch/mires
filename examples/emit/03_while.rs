#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// A `while` loop: the `for (;;) { if (c) .. else break }` peephole in emit.rs.

fn triangle(n: i32) -> i32 {
    let mut i = 0;
    let mut acc = 0;
    while i < n {
        acc = acc + i;
        i = i + 1;
    }
    acc
}

#[no_mangle]
fn rust_entry() {
    print_i32(triangle(5));
    print_i32(triangle(0));
}
