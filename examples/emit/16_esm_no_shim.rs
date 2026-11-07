#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// A module that never calls the runtime shim, so the `__rt` import is dropped.
//
// The import is not a root: it survives only while some reachable item mentions
// what it binds. Nothing here prints, so nothing does, and the golden below has
// no import line at all. The export clause still names both exported items, in
// sorted order.

#[no_mangle]
fn add_one(x: i32) -> i32 {
    x + 1
}

#[no_mangle]
fn rust_entry() {}
