#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// A labeled break out of a nested loop: the case a JS label is actually needed for.

fn find(limit: i32, target: i32) -> i32 {
    let mut i = 0;
    let mut found = -1;
    'outer: while i < limit {
        let mut j = 0;
        while j < limit {
            if i * j == target {
                found = i * 10 + j;
                break 'outer;
            }
            j = j + 1;
        }
        i = i + 1;
    }
    found
}

#[no_mangle]
fn rust_entry() {
    print_i32(find(6, 12));
    print_i32(find(3, 99));
}
