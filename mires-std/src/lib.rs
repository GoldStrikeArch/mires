//! Thin `std` sysroot crate for the `mires` (Rust-to-JavaScript) compiler.
//!
//! Provides everyday Rust standard library APIs over `core`, `alloc`, and the
//! minimal JavaScript runtime shim (`runtime/shim.js`), without requiring
//! `#![no_std]`, `extern crate alloc`, or a manual `#[panic_handler]` in user
//! crates.

#![no_std]
#![allow(internal_features)]
#![feature(
    lang_items,
    never_type,
    rustc_attrs,
    core_intrinsics,
    panic_internals,
    const_eval_select
)]
// Allow unused items across std APIs that exist for compatibility with upstream `std`.
#![allow(dead_code)]

#[doc(hidden)]
pub extern crate alloc as alloc_crate;
#[doc(hidden)]
pub extern crate core as core_crate;

mod macros;
mod num_float;
mod panicking;
mod sys;

pub mod collections;
pub mod env;
pub mod ffi;
pub mod fs;
pub mod io;
pub mod net;
pub mod panic;
pub mod path;
pub mod prelude;
pub mod process;
pub mod rt;
pub mod sync;
pub mod thread;
pub mod time;

// Re-export `core` modules.
pub use core_crate::any;
pub use core_crate::array;
pub use core_crate::ascii;
pub use core_crate::cell;
pub use core_crate::char;
pub use core_crate::clone;
pub use core_crate::cmp;
pub use core_crate::convert;
pub use core_crate::default;
pub use core_crate::error;
pub use core_crate::future;
pub use core_crate::hint;
pub use core_crate::i128;
pub use core_crate::i16;
pub use core_crate::i32;
pub use core_crate::i64;
pub use core_crate::i8;
pub use core_crate::isize;
pub use core_crate::iter;
pub use core_crate::marker;
pub use core_crate::mem;
pub use core_crate::num;
pub use core_crate::ops;
pub use core_crate::option;
pub use core_crate::pin;
pub use core_crate::primitive;
pub use core_crate::ptr;
pub use core_crate::result;
pub use core_crate::u128;
pub use core_crate::u16;
pub use core_crate::u32;
pub use core_crate::u64;
pub use core_crate::u8;
pub use core_crate::usize;

// Re-export `alloc` modules.
pub use alloc_crate::alloc;
pub use alloc_crate::borrow;
pub use alloc_crate::boxed;
pub use alloc_crate::fmt;
pub use alloc_crate::format;
pub use alloc_crate::rc;
pub use alloc_crate::slice;
pub use alloc_crate::str;
pub use alloc_crate::string;
pub use alloc_crate::vec;

/// Constants and math methods for `f32`.
pub mod f32 {
    pub use core_crate::f32::*;
}

/// Constants and math methods for `f64`.
pub mod f64 {
    pub use core_crate::f64::*;
}

/// Hashing traits plus the default `HashMap` / `HashSet` hasher.
pub mod hash {
    pub use crate::collections::hash_map::{DefaultHasher, RandomState};
    pub use core_crate::hash::*;
}

/// Types and traits for working with asynchronous tasks.
pub mod task {
    pub use alloc_crate::task::*;
    pub use core_crate::task::*;
}

// Re-export standard macros at the crate root.
pub use core_crate::{
    assert, assert_eq, assert_ne, cfg, cfg_select, column, compile_error, concat, debug_assert,
    debug_assert_eq, debug_assert_ne, env, file, format_args, include, include_bytes, include_str,
    line, matches, module_path, option_env, stringify, todo, unimplemented, unreachable, write,
    writeln,
};

/// Default panic handler for `std` crates compiled by `mires`.
///
/// Extracts a static string message when available so programs without runtime
/// formatting do not pull `core::fmt` into the emitted JavaScript.
#[panic_handler]
fn panic_handler(info: &core_crate::panic::PanicInfo<'_>) -> ! {
    let msg = info.message().as_str().unwrap_or("explicit panic");
    panicking::panic_with_static_str(msg)
}
