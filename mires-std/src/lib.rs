//! Thin `std` sysroot crate for the `mires` (Rust-to-JavaScript) compiler.
//!
//! Provides everyday Rust standard library APIs over `core`, `alloc`, and the
//! minimal JavaScript runtime shim (`runtime/shim.js`), without requiring
//! `#![no_std]`, `extern crate alloc`, or a manual `#[panic_handler]` in user
//! crates.

#![no_std]
#![allow(internal_features)]
#![feature(
    alloc_error_handler,
    allocator_api,
    const_eval_select,
    core_intrinsics,
    custom_test_frameworks,
    lang_items,
    never_type,
    panic_internals,
    rustc_attrs,
    test,
    try_reserve_kind
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

pub mod backtrace;
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
pub use alloc_crate::borrow;
pub use alloc_crate::boxed;
pub use alloc_crate::fmt;
pub use alloc_crate::format;
pub use alloc_crate::rc;
pub use alloc_crate::slice;
pub use alloc_crate::str;
pub use alloc_crate::string;
pub use alloc_crate::vec;

/// Memory allocation APIs, extending `alloc::alloc` with [`System`].
pub mod alloc {
    pub use alloc_crate::alloc::*;

    use core_crate::ptr::NonNull;

    /// The default system allocator backed by the Mires JS runtime heap.
    #[derive(Debug, Default, Copy, Clone)]
    pub struct System;

    unsafe impl GlobalAlloc for System {
        #[inline]
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            unsafe { alloc_crate::alloc::alloc(layout) }
        }

        #[inline]
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            unsafe { alloc_crate::alloc::alloc_zeroed(layout) }
        }

        #[inline]
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { alloc_crate::alloc::dealloc(ptr, layout) }
        }

        #[inline]
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            unsafe { alloc_crate::alloc::realloc(ptr, layout, new_size) }
        }
    }

    unsafe impl Allocator for System {
        #[inline]
        fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
            Global.allocate(layout)
        }

        #[inline]
        fn allocate_zeroed(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
            Global.allocate_zeroed(layout)
        }

        #[inline]
        unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
            unsafe { Global.deallocate(ptr, layout) }
        }

        #[inline]
        unsafe fn grow(
            &self,
            ptr: NonNull<u8>,
            old_layout: Layout,
            new_layout: Layout,
        ) -> Result<NonNull<[u8]>, AllocError> {
            unsafe { Global.grow(ptr, old_layout, new_layout) }
        }

        #[inline]
        unsafe fn grow_zeroed(
            &self,
            ptr: NonNull<u8>,
            old_layout: Layout,
            new_layout: Layout,
        ) -> Result<NonNull<[u8]>, AllocError> {
            unsafe { Global.grow_zeroed(ptr, old_layout, new_layout) }
        }

        #[inline]
        unsafe fn shrink(
            &self,
            ptr: NonNull<u8>,
            old_layout: Layout,
            new_layout: Layout,
        ) -> Result<NonNull<[u8]>, AllocError> {
            unsafe { Global.shrink(ptr, old_layout, new_layout) }
        }
    }
}

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
    assert, assert_eq, assert_matches, assert_ne, cfg, cfg_select, column, compile_error, concat,
    debug_assert, debug_assert_eq, debug_assert_matches, debug_assert_ne, env, file, format_args,
    include, include_bytes, include_str, line, matches, module_path, option_env, stringify, todo,
    unimplemented, unreachable, write, writeln,
};

/// Default panic handler for `std` crates compiled by `mires`.
///
/// Uses the static-string fast path when available and formats dynamic panic
/// messages (such as `assert_eq!` or bounds checks) so `catch_unwind` and
/// `#[should_panic(expected = "...")]` observe the exact message.
#[panic_handler]
fn panic_handler(info: &core_crate::panic::PanicInfo<'_>) -> ! {
    if let Some(msg) = info.message().as_str() {
        panicking::panic_with_static_str(msg)
    } else {
        panicking::panic_with_fmt(format_args!("{}", info.message()))
    }
}
