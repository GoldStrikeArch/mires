#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// Per island chunking (`-Cllvm-args=js-chunk=..`, see 19_chunks.args).
//
// Two entry points, each asked for as a chunk of its own, over a crate where some
// items belong to one island, some to the other, and some to both. What the
// goldens hold:
//
//   19_chunks.js.expected     the whole program, written to the `-o` path exactly
//                             as it is without any chunk asked for. A split is an
//                             extra emission, not a replacement.
//   19_chunks.a.js.expected   island A: `only_a`, and an import of what it shares
//   19_chunks.b.js.expected   island B: `only_b`, and the same import
//   19_chunks.shared.js.expected  what both islands reach, exported by name
//
// So the split is pinned three ways at once: A's file must not contain B's code,
// the common item must appear once and in `shared.js`, and the three files have to
// run together under node, which is what proves the imports resolve and the
// exported names match.
//
// `rust_entry` is a root no chunk was asked for, so it appears in the whole program
// and in none of the chunks. That is the other half of the split being a decision
// about entry points: a chunk holds what its own entry reaches, and nothing else.
// It is also what lets scripts/module-test.sh run this crate, which compiles it
// without any chunk asked for at all.

/// Reached from both islands, so it belongs to neither.
fn shared_add(x: i32) -> i32 {
    x + x
}

/// Reached from island A only.
fn only_a(x: i32) -> i32 {
    shared_add(x) + 1
}

/// Reached from island B only.
fn only_b(x: i32) -> i32 {
    shared_add(x) - 1
}

#[no_mangle]
fn __island_a() {
    print_str("a");
    print_i32(only_a(20));
}

#[no_mangle]
fn __island_b() {
    print_str("b");
    print_i32(only_b(20));
}

#[no_mangle]
fn rust_entry() {
    __island_a();
    __island_b();
}
