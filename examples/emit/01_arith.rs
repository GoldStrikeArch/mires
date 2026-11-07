#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// Masking arithmetic: every operator wraps back into its Rust type.

fn mix(a: i32, b: i32) -> i32 {
    let sum = a + b;
    let scaled = sum * 3;
    let shifted = scaled << 2;
    shifted ^ a
}

fn bytes(x: u8, y: u8) -> u8 {
    let wide = x + y;
    wide << 3
}

#[no_mangle]
fn rust_entry() {
    print_i32(mix(7, 11));
    print_i32(bytes(200, 100) as i32);
}
