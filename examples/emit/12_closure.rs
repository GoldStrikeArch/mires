#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// A closure: a capture struct, a `call_once` shim and the splatted argument tuple.

fn apply<F: Fn(i32) -> i32>(f: F, x: i32) -> i32 {
    f(x)
}

#[no_mangle]
fn rust_entry() {
    let base = 10;
    let add_base = |x: i32| x + base;
    print_i32(apply(add_base, 5));
    print_i32(apply(|x: i32| x * x, 6));
}
