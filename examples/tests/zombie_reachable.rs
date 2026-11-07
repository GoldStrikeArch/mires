// A zombie the program can reach is an error, reported where it was recorded.
//
// `rust_entry` calls `align_down`, so link-time reachability keeps the item, and the zombie its
// `ptr_mask` recorded turns into a diagnostic naming this file — plus the chain of items that made
// it reachable.
//
// `ptr_mask` is deliberately unsupported, and stays that way: it asks for the *bits* of a pointer,
// and a pointer here is a buffer and a key into it rather than an address, so there are no bits to
// mask. Its neighbours in the same family — `arith_offset`, `ptr_offset_from`, `copy`,
// `write_bytes`, `compare_bytes` — are implemented as of the stage 1.5 pointer model, and this
// fixture was written against `arith_offset` until then. It replaced an earlier i64 fixture that
// stage 1's BigInt support made compile; the pattern is the same each time.
//
// `repack` is the second half of the fixture, and the second permanent gap: a cast between two
// pointees that are neither the same size nor a byte. A pointer is a buffer and a key into it, so
// a cast can only ever re-key an existing buffer — reading a three field record out of a buffer of
// two field ones would need elements that are not there. `cast_pointer` names both types when it
// gives up, which is what this pins.
//
// This test is expected to FAIL to compile: `zombie_reachable.expect_fail` lists the substrings
// the compiler output has to contain.

#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// `ptr_mask` is a *safe* intrinsic — masking a pointer cannot make it point anywhere new — so it
// is declared without `unsafe`, which is what the compiler's own list says it is.
#[rustc_intrinsic]
fn ptr_mask<T>(ptr: *const T, mask: usize) -> *const T;

#[inline(never)]
fn align_down(p: *const i32) -> *const i32 {
    ptr_mask(p, 3)
}

pub struct Pair {
    a: i32,
    b: i32,
}

pub struct Triple {
    a: i32,
    b: i32,
    c: i32,
}

/// A size changing aggregate pun: eight bytes read as twelve.
#[inline(never)]
fn repack(p: *const Pair) -> *const Triple {
    p as *const Triple
}

#[no_mangle]
fn rust_entry() {
    let x = 7;
    align_down(&x as *const i32);
    let pair = Pair { a: 1, b: 2 };
    repack(&pair as *const Pair);
    print_i32(0);
}
