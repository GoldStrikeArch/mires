#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// Field chains through nested structs, by value and behind a reference.

struct Inner {
    a: i32,
    b: i32,
}

struct Outer {
    left: Inner,
    right: Inner,
}

fn total(o: &Outer) -> i32 {
    o.left.a + o.left.b + o.right.a + o.right.b
}

fn bump(o: &mut Outer, by: i32) {
    o.right.b = o.right.b + by;
}

#[no_mangle]
fn rust_entry() {
    let mut o = Outer { left: Inner { a: 1, b: 2 }, right: Inner { a: 3, b: 4 } };
    print_i32(total(&o));
    bump(&mut o, 10);
    print_i32(total(&o));
}
