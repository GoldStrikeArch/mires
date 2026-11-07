//! A gap that is a run time refusal rather than a zombie: a zeroed heap block read as a type whose
//! zero this backend cannot spell.
//!
//! `alloc_zeroed` hands back a block of zero *bytes*, and the retype has to turn those into zero
//! *elements* -- which is not the same thing. A zero `bool` is `false`, a zero niched `Option` is
//! `{ $t: 0 }`, a zero struct is an object with a zero in every field, and only the compiler knows
//! which. So the cast carries a factory for one (`CONTRACT.md`, "Allocation").
//!
//! A **fat** pointer has no such factory. `&[T]` is `{ buf, off, len }`, and there is no buffer for
//! an all-zero one to name: a slice with no provenance is not a value this model has. `vec![0; n]`
//! and `vec![false; n]` are the shapes that do work, and this file is the edge of that.
//!
//! If a later round finds a meaning for the zero of every type, this test starts failing by
//! *succeeding*.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::alloc::{Layout, alloc_zeroed, dealloc};

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let layout = unsafe { Layout::from_size_align_unchecked(8, 4) };

    // A zeroed block read as numbers is fine: the zero of an `i32` is the byte zero, so the fill
    // has something to put there.
    let numbers = unsafe { alloc_zeroed(layout) };
    let words = numbers as *mut i32;
    print_i32(unsafe { *words });
    unsafe { dealloc(numbers, layout) };

    // The same bytes as a slice reference have no zero to be filled with.
    let block = unsafe { alloc_zeroed(layout) };
    let slices = block as *mut &'static [i32];
    print_usize(unsafe { *slices }.len());

    // Unreachable: the cast above throws.
    unsafe { dealloc(block, layout) };
    print_i32(999);
}
