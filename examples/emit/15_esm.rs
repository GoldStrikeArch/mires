#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// ES module output (`-Cllvm-args=js-modules=esm`, see 15_esm.args).
//
// The import comes first, the items are ordered exactly as a script build orders
// them, and the export clause comes last. Every line between the two is byte for
// byte what the same crate compiled as a script prints; scripts/module-test.sh
// is what holds that.

fn double(x: i32) -> i32 {
    x + x
}

#[no_mangle]
fn rust_entry() {
    print_i32(double(21));
    print_str("module");
}
