#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// The degenerate split: one chunk, over the crate's only exported item.
//
// Nothing can be shared when there is only one chunk to share it with, so no
// shared chunk is written and the one chunk holds the whole program. This fixture
// carries 20_chunk_one.degenerate, which is what makes the suite check that the
// chunk and the unsplit output are the same bytes apart from the header comment
// that names the file. That equality is the property worth pinning: asking for a
// split cannot change what a program means, and a build tool can ask for chunks
// before it knows whether there is anything to split.
//
// The equality needs the chunk's entry to be the program's only root, so the entry
// asked for is `rust_entry` and there is nothing else exported. Any second root
// would be a part of the program the one chunk is not asked for, and rightly left
// out of it.

fn triple(x: i32) -> i32 {
    x + x + x
}

#[no_mangle]
fn rust_entry() {
    print_str("one");
    print_i32(triple(14));
}
