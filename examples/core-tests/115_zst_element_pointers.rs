//! Every pointer to a zero-sized element is one address, whichever way it was made.
//!
//! An element of `[Z; N]` or `[Z]` has no storage (`114_zst_aggregate_drops`), so a pointer to it
//! cannot name a slot. What it is instead is the address the whole run shares: the array's own
//! pointer, or the data pointer the slice was built from, which is what `as_ptr` answers and what
//! `from_raw_parts` round trips. So `&a[i]`, `a.as_ptr()`, `s.as_ptr()`, `s.first()`, an iterator's
//! item, `as_ptr().add(n)` and a `Vec<Z>`'s own pointer all compare equal under `ptr::eq`,
//! `addr_eq` and a `usize` cast, none of them is null, and every one is aligned (CONTRACT.md,
//! "Pointers"). Only relations Rust defines are printed: two different zero-sized arrays may or
//! may not share an address, so that is never asked.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::vec::Vec;
use core::hint::black_box;
use core::ptr::{self, NonNull};

static mut DROPPED: u32 = 0;

/// A zero-sized element that counts its drops.
struct Z;

impl Drop for Z {
    fn drop(&mut self) {
        unsafe { DROPPED += 1 }
    }
}

/// A zero-sized element with a large alignment, so alignment predicates are asked of something.
#[repr(align(64))]
struct Wide;

/// An array, its elements, and the slice unsized from it.
#[inline(never)]
fn array_and_slice() {
    let mut a = black_box([Z, Z]);
    print_bool(ptr::eq(&a[black_box(0)], &a[black_box(1)]));
    print_bool(ptr::eq(&a[black_box(0)], &a[black_box(0)]));
    print_bool(ptr::eq(&a[black_box(0)], &a as *const [Z; 2] as *const Z));
    let first = &a[black_box(0)] as *const Z;
    let second = &a[black_box(1)] as *const Z;
    print_usize(second.addr().wrapping_sub(first.addr()));
    print_bool(first.is_null());
    {
        let s: &[Z] = &a;
        print_bool(ptr::eq(s.as_ptr(), &a[black_box(0)]));
        print_bool(black_box(s.as_ptr()).is_null());
        print_bool(black_box(s.as_ptr()).cast::<u8>().is_null());
        print_bool(s.as_ptr().is_aligned());
        print_bool(ptr::eq(s.as_ptr(), a.as_ptr()));
        print_bool(ptr::eq(unsafe { s.as_ptr().add(1) }, s.as_ptr()));
        print_bool(ptr::eq(s.as_ptr().wrapping_add(1), s.as_ptr()));
        print_bool(ptr::eq(s.first().unwrap(), s.as_ptr()));
        print_bool(ptr::eq(s.iter().next().unwrap(), &a[black_box(1)]));
        print_bool(ptr::addr_eq(s as *const [Z], &a[black_box(1)] as *const Z));
        print_bool(s.as_ptr() as usize == second as usize);
        let rebuilt = unsafe { core::slice::from_raw_parts(s.as_ptr(), s.len()) };
        print_bool(ptr::eq(rebuilt.as_ptr(), s.as_ptr()));
        print_usize(rebuilt.len());
    }
    let p = a.as_mut_ptr();
    print_bool(ptr::eq(p, &a[black_box(1)]));
    drop(a);
    print_str("array");
}

/// A `Vec<Z>` never allocates, so its pointer is a dangling address, and its elements are there.
#[inline(never)]
fn vector() {
    let mut v: Vec<Z> = Vec::new();
    v.push(Z);
    v.push(Z);
    print_bool(ptr::eq(v.as_ptr(), &v[black_box(1)]));
    print_bool(ptr::eq(v.as_slice().as_ptr(), v.as_ptr()));
    print_bool(black_box(v.as_ptr()).is_null());
    let dangling = NonNull::<Z>::dangling().as_ptr();
    let parts = unsafe { core::slice::from_raw_parts(dangling, 3) };
    print_bool(ptr::eq(parts.as_ptr(), dangling));
    print_bool(ptr::eq(&parts[black_box(2)], dangling));
    print_usize(parts.len());
    drop(v);
    print_str("vec");
}

/// A zero-sized element whose type asks for 64-byte alignment.
#[inline(never)]
fn aligned() {
    let w = black_box([Wide, Wide, Wide]);
    let p = &w[black_box(2)] as *const Wide;
    print_bool(p.is_aligned());
    print_bool(p.addr() % 64 == 0);
    print_bool(!p.is_null());
    print_bool(ptr::eq(p, w.as_ptr()));
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    array_and_slice();
    vector();
    aligned();
    print_u32(unsafe { DROPPED });
}
