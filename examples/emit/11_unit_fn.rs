#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// Functions returning `()`: nothing to return, and nothing to declare for it.

fn shout(x: i32) {
    print_i32(x);
    print_i32(x + 1);
}

fn nothing() {}

#[no_mangle]
fn rust_entry() {
    shout(41);
    nothing();
    print_i32(0);
}
