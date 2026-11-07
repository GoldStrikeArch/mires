#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// A chain of calls, each feeding the next: one temporary per link.

fn double(x: i32) -> i32 {
    x * 2
}

fn inc(x: i32) -> i32 {
    x + 1
}

fn square(x: i32) -> i32 {
    x * x
}

fn pipeline(x: i32) -> i32 {
    square(inc(double(inc(x))))
}

#[no_mangle]
fn rust_entry() {
    print_i32(pipeline(3));
}
