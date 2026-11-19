//! `compare_bytes` over the pointer a heap block was allocated with, after the block has been
//! retyped to wider elements: what `[u16] == [u16]` reaches once rustc's MIR inliner has run.
//!
//! The bytewise `PartialEq` of a slice of integers calls `compare_bytes(a as *const u8, b as *const
//! u8, len * size_of::<T>())`. `Vec::as_ptr` is a cast of the `*mut u8` that `RawVecInner` stores,
//! so with the bodies inlined into one, rustc's GVN folds `NonNull<u8> -> *const u16 -> *const u8`
//! into a single cast of the stored pointer, and the intrinsic is handed the allocation's own
//! pointer rather than the byte view `as *const u8` makes of an element pointer. Here that pointer
//! is keyed in the block's elements, not in bytes (`CONTRACT.md`, "Allocation"), so the byte count
//! was read as an element count: two equal-length slices whose capacity was their length threw
//! "`compare_bytes` over 4 elements at 0 reaches past the end of a buffer of 2", and two with spare
//! capacity compared the stale or empty slots past their length as well.
//!
//! The first half calls the intrinsic on such a pointer directly, which needs no inliner, so the
//! suite pins it in every mode; the second is the slice comparisons themselves, which reach it only
//! under `JSC_RUST_MIR_INLINE=on scripts/test.sh 345_byte_compare_heap_pointer` (the suite runs with
//! the inliner off). Only equality is printed: the sign of an unequal answer is the order of two
//! elements here and of two bytes natively.

#![no_std]
#![no_main]
#![feature(core_intrinsics)]
#![allow(internal_features)]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::alloc::{alloc, dealloc, Layout};
use alloc::vec::Vec;
use core::intrinsics::compare_bytes;

/// A heap block of `values.len()` `u16`s, filled, and the `*mut u8` it was allocated as.
fn block(values: &[u16]) -> (*mut u8, Layout) {
    let layout = Layout::array::<u16>(values.len()).unwrap();
    unsafe {
        let bytes = alloc(layout);
        let words = bytes as *mut u16;
        for (index, value) in values.iter().enumerate() {
            words.add(index).write(*value);
        }
        (bytes, layout)
    }
}

#[inline(never)]
fn same_bytes(a: *const u8, b: *const u8, count: usize) -> bool {
    unsafe { compare_bytes(a, b, count) == 0 }
}

#[inline(never)]
fn equal_units(a: &[u16], b: &[u16]) -> bool {
    a == b
}

#[inline(never)]
fn equal_words(a: &[u32], b: &[u32]) -> bool {
    a == b
}

/// A vector of exactly `values.len()` capacity.
fn exact(values: &[u16]) -> Vec<u16> {
    let mut out = Vec::with_capacity(values.len());
    out.extend_from_slice(values);
    out
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let (a, a_layout) = block(&[0x6f, 0x72]);
    let (b, b_layout) = block(&[0x6f, 0x72]);
    let (c, c_layout) = block(&[0x6f, 0x78]);
    print_bool(same_bytes(a, b, 4));
    print_bool(same_bytes(a, c, 4));
    print_bool(same_bytes(a, c, 2));
    unsafe {
        dealloc(a, a_layout);
        dealloc(b, b_layout);
        dealloc(c, c_layout);
    }

    // Capacity equal to the length: the byte count used to run off the end of the block.
    let or = exact(&[0x6f, 0x72]);
    print_bool(equal_units(&or, &exact(&[0x6f, 0x72])));
    print_bool(equal_units(&or, &exact(&[0x6f, 0x78])));
    print_bool(or == exact(&[0x6f, 0x72]));

    // Spare capacity, one with a stale element left behind past its length: only the first `len`
    // elements may be compared.
    let mut stale: Vec<u16> = Vec::with_capacity(4);
    stale.extend_from_slice(&[1, 2, 9]);
    stale.pop();
    let mut fresh: Vec<u16> = Vec::with_capacity(4);
    fresh.extend_from_slice(&[1, 2]);
    print_bool(equal_units(&stale, &fresh));
    print_bool(stale == fresh);

    let mut words: Vec<u32> = Vec::with_capacity(3);
    words.extend_from_slice(&[7, 8, 9]);
    words.truncate(2);
    print_bool(equal_words(&words, &[7, 8]));
    print_bool(equal_words(&words, &[7, 9]));
}
