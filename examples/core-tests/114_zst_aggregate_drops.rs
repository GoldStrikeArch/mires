//! Zero-sized aggregates keep the storage they elide, and every element is still destroyed.
//!
//! A zero-sized value is `undefined` (CONTRACT.md, "Value representation"), and so is an array of
//! zero-sized elements: `[Z; 2000]` allocates nothing. Its elements are still there logically --
//! the array type counts them, a slice's `len` counts them -- and Rust owes each one its
//! destructor, exactly `len` times, in index order. Compiler drop glue walks the elements through
//! `&raw mut (*slice)[i]`, and such an element has no storage to name (`JsPlace::Unit`): its value
//! is `undefined` and its pointer is the slice's data pointer, rather than a read of `buf[off + i]`
//! out of a buffer that does not exist, which was a `TypeError` in every case below.
//!
//! Covered: a local, a returned array, a by-value and a by-reference parameter, a `&mut [Z]`,
//! `move` closures (`Fn` and `FnOnce`), `Vec<[Z; N]>`, `[[u8; 0]; N]`, zero-length arrays, partial
//! moves out of an array (a pattern hole and a `rest @ ..`), whole-array and element replacement,
//! and `[Z; 2000]`, whose output has to stay small (`.absent`, `.maxbytes`).
//!
//! `zst_host_throw` is the exceptional path: `scripts/cleanup-smoke.sh` makes its host call throw
//! and checks that cleanup still drops every element exactly once. `rust_entry` runs it without the
//! throw. Each case is a function of its own, so that no one body nests every call of the file
//! under the per-call exception handling `js-cleanup=on` adds.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::vec::Vec;
use core::hint::black_box;

unsafe extern "C-unwind" {
    fn host_throw();
}

/// A zero-sized element whose destructor prints `K`, so the elements of different arrays can be
/// told apart while every one of them stays zero-sized.
struct Z<const K: u32>;

impl<const K: u32> Drop for Z<K> {
    fn drop(&mut self) {
        print_u32(K);
    }
}

/// A zero-sized element that only counts, for an array too long to print.
struct Counted;

impl Drop for Counted {
    fn drop(&mut self) {
        unsafe { DROPPED += 1 }
    }
}

static mut DROPPED: u32 = 0;

fn dropped() -> u32 {
    unsafe { DROPPED }
}

#[inline(never)]
fn make() -> [Z<2>; 3] {
    [Z, Z, Z]
}

#[inline(never)]
fn take(values: [Z<3>; 2]) {
    print_usize(black_box(&values).len());
}

#[inline(never)]
fn look(values: &[Z<4>; 2]) -> usize {
    values.len() + values.iter().count()
}

#[inline(never)]
fn through_slice(values: &mut [Z<5>]) -> usize {
    let last = values.len() - 1;
    values.swap(0, last);
    black_box(&mut values[black_box(1)]);
    values.len() * 10 + values.iter_mut().count()
}

#[inline(never)]
fn small() -> [[u8; 0]; 3] {
    [[]; 3]
}

#[inline(never)]
fn crowd() -> [Counted; 2000] {
    [const { Counted }; 2000]
}

/// A local: its length read through a reference, its elements dropped at the end of its scope.
/// Then a returned array, dropped explicitly, and a by-value parameter, dropped by the callee.
#[inline(never)]
fn owned() {
    {
        let local = [Z::<1>, Z::<1>];
        print_usize(black_box(&local).len());
    }
    print_str("local");
    let made = make();
    print_usize(black_box(made.len()));
    drop(made);
    print_str("returned");
    take([Z, Z]);
    print_str("by value");
}

/// By reference: an array, and a mutable slice that is swapped, indexed and iterated.
#[inline(never)]
fn borrowed() {
    let shared = black_box([Z::<4>, Z::<4>]);
    print_usize(look(black_box(&shared)));
    drop(shared);
    let mut slice = [Z::<5>, Z::<5>, Z::<5>];
    print_usize(through_slice(black_box(&mut slice)));
    drop(slice);
    print_str("by reference");
}

/// `move` closures: one that borrows its capture and is dropped with it, one that consumes it.
#[inline(never)]
fn closures() {
    let captured = black_box([Z::<6>, Z::<6>]);
    let count = move || black_box(&captured).len();
    print_usize(count());
    drop(black_box(count));
    let consumed = [Z::<6>, Z::<6>, Z::<6>];
    let consume = move || drop(consumed);
    consume();
    print_str("closure");
}

/// `Vec<[Z; N]>`: a vector of zero-sized elements that are themselves arrays.
#[inline(never)]
fn vectors() {
    let mut rows: Vec<[Z<7>; 2]> = Vec::new();
    rows.push(black_box([Z, Z]));
    rows.push([Z, Z]);
    print_usize(rows.len());
    let popped = rows.pop();
    print_bool(popped.is_some());
    drop(popped);
    drop(black_box(rows));
    print_str("vec");
}

/// `[[u8; 0]; N]` and zero-length arrays: indexing, measuring, and nothing to drop or visit.
#[inline(never)]
fn empties() {
    let small = black_box(small());
    print_usize(small.len() + small[black_box(2)].len());
    let empty: &[u8] = &small[black_box(1)];
    print_bool(empty.is_empty());
    let none: [Z<8>; 0] = [];
    print_usize(black_box(&none).len());
    let nothing: &[Z<8>] = &none;
    print_bool(nothing.iter().next().is_none());
    drop(none);
    let numbers: [u32; 0] = black_box([]);
    print_usize(numbers.len() + numbers.iter().count());
    print_str("zero length");
}

/// Partial moves: what is left behind is dropped with the array, the moved part on its own.
#[inline(never)]
fn partial() {
    {
        let row = [Z::<9>, Z::<9>, Z::<9>];
        let [_, middle, _] = row;
        print_str("moved");
        drop(middle);
        print_str("middle");
    }
    {
        let row = [Z::<10>, Z::<10>, Z::<10>];
        let [first, rest @ ..] = row;
        print_usize(rest.len());
        drop(rest);
        print_str("rest");
        drop(first);
    }
    print_str("partial");
}

/// Replacement: assigning over an array drops its old elements first, and so does assigning over
/// one element.
#[inline(never)]
fn replaced() {
    let mut slot = [Z::<11>, Z::<11>];
    print_usize(black_box(&slot).len());
    slot = black_box([Z, Z]);
    print_str("replaced");
    slot[black_box(1)] = Z;
    print_str("element");
    drop(slot);
}

/// `[Z; 2000]`: two thousand drops, and no two-thousand-element array in the output.
#[inline(never)]
fn many() {
    let many = black_box(crowd());
    black_box(&many[black_box(1999)]);
    print_usize(many.len());
    print_usize(many.iter().count());
    drop(many);
    print_u32(dropped());
}

/// The exceptional path. With `throwing` the host call throws, and cleanup must still drop `rows`
/// and then `held`, every element exactly once, before the exception leaves Rust.
#[unsafe(no_mangle)]
pub fn zst_host_throw(throwing: bool) -> u32 {
    let held = make();
    let mut rows: Vec<[Z<7>; 2]> = Vec::new();
    rows.push([Z, Z]);
    if throwing {
        unsafe { host_throw() };
    }
    drop(rows);
    drop(held);
    12
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    owned();
    borrowed();
    closures();
    vectors();
    empties();
    partial();
    replaced();
    many();
    print_u32(zst_host_throw(false));
}
