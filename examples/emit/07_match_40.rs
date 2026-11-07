#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// A forty arm match: past the point where a chain of `if`s would be the wrong shape.

fn pick(n: i32) -> i32 {
    match n {
        0 => 1,
        1 => 8,
        2 => 15,
        3 => 22,
        4 => 29,
        5 => 36,
        6 => 43,
        7 => 50,
        8 => 57,
        9 => 64,
        10 => 71,
        11 => 78,
        12 => 85,
        13 => 92,
        14 => 99,
        15 => 106,
        16 => 113,
        17 => 120,
        18 => 127,
        19 => 134,
        20 => 141,
        21 => 148,
        22 => 155,
        23 => 162,
        24 => 169,
        25 => 176,
        26 => 183,
        27 => 190,
        28 => 197,
        29 => 204,
        30 => 211,
        31 => 218,
        32 => 225,
        33 => 232,
        34 => 239,
        35 => 246,
        36 => 253,
        37 => 260,
        38 => 267,
        39 => 274,
        _ => -1,
    }
}

#[no_mangle]
fn rust_entry() {
    print_i32(pick(0));
    print_i32(pick(1));
    print_i32(pick(17));
    print_i32(pick(39));
    print_i32(pick(40));
}
