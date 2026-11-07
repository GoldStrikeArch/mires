#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// `loop` with a conditional `break`, plus a `continue`.

fn first_multiple(of: i32, above: i32) -> i32 {
    let mut candidate = above;
    loop {
        candidate = candidate + 1;
        if candidate % of != 0 {
            continue;
        }
        break;
    }
    candidate
}

#[no_mangle]
fn rust_entry() {
    print_i32(first_multiple(7, 10));
}
