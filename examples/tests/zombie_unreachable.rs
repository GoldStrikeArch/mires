// A zombie nothing reaches costs nothing.
//
// `wide_sum` is `pub` and non-generic, so rustc's monomorphization collector roots it — a
// downstream crate could call it — and the backend lowers it. Its `i64` arithmetic is something
// the backend cannot lower yet, so lowering records a zombie.
//
// Nothing in *this program* calls it, and it carries no `#[no_mangle]`/`#[export_name]`/`#[used]`
// attribute, so it is not a link root either: reachability drops the item and the zombie with it,
// and this file compiles clean. The companion check is `zombie_unreachable.absent` — the emitted
// JavaScript must not mention `wide_sum` at all.

#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

/// Codegenned, never called. Its body is a zombie; the item must not survive to the output.
#[inline(never)]
pub fn wide_sum(a: i64, b: i64) -> i64 {
    a + b
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

/// The same again for a pointer cast the stage 1.5 matrix rejects — a pointee of one size read as
/// a pointee of another. Unreachable, so it costs nothing either; `zombie_unreachable.absent`
/// checks that `repack` is not in the output.
#[inline(never)]
pub fn repack(p: *const Pair) -> *const Triple {
    p as *const Triple
}

#[no_mangle]
fn rust_entry() {
    print_i32(7);
}
