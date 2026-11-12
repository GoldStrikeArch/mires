//! NEW REFUSAL of a program main compiled and ran correctly (safe Rust): a closure reified as a
//! `fn` pointer and cast to `*const ()` inside a constant, observed only through safe operations.
//! Native (-O0, -O3): false true. Main (ea5604f), all four modes: false true.
//! e2245df, all four modes: compile error "a closure stored as a `*const ()`, which is not a `fn`
//! pointer, in constant data is not supported by rustc_codegen_js".
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

const P0: *const () = (|| 5u32) as fn() -> u32 as *const ();

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_bool(P0.is_null());
    print_bool(black_box(P0) == black_box(P0));
}
