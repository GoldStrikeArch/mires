#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// The shape of the stage 1.5 pointer model, pinned.
//
// A pointer is a slot — a buffer and a key into it — so a dereference is `p.buf[p.off]`, an
// assignable JavaScript place, and an offset is a new record with `off + n`. The golden is here
// to keep three things readable rather than merely correct:
//
//   * `bump` reads and writes through a pointer as one indexed assignment, with no helper call;
//   * `nth` folds the offset into the index, so `*p.add(n)` prints as `p.buf[p.off + n]` rather
//     than building an intermediate record;
//   * `sum` is the loop every slice iterator in `core` becomes once it is monomorphized — a
//     moving pointer, a fixed end, and a remaining length that is the difference of two offsets.
//     `core`'s own `slice::Iter` is this loop with the same three operations in it.
//
// `value` is a local whose address is taken and whose value is a JavaScript number, so it is
// declared as a one element array and used as `value[0]`: that is the boxing the model needs to
// give a primitive a place to be pointed at. The array in `rust_entry` needs no such thing — it is
// a buffer already.

#[rustc_intrinsic]
const unsafe fn offset<Ptr, Delta>(dst: Ptr, offset: Delta) -> Ptr;

#[rustc_intrinsic]
const unsafe fn ptr_offset_from<T>(ptr: *const T, base: *const T) -> isize;

/// A slot dereferenced on both sides of an assignment.
#[inline(never)]
fn bump(target: *mut i32, by: i32) {
    unsafe {
        *target = *target + by;
    }
}

/// A slot moved by an element count, then read.
#[inline(never)]
fn nth(base: *const i32, index: isize) -> i32 {
    unsafe { *offset(base, index) }
}

/// The loop a slice iterator compiles to.
#[inline(never)]
fn sum(start: *const i32, end: *const i32) -> i32 {
    let mut at = start;
    let mut total = 0;
    while unsafe { ptr_offset_from(end, at) } > 0 {
        total = total + unsafe { *at };
        at = unsafe { offset(at, 1isize) };
    }
    total
}

#[no_mangle]
fn rust_entry() {
    let mut value = 1;
    bump(&mut value as *mut i32, 41);
    print_i32(value);

    let values = [10, 20, 30, 40];
    let base = &values[0] as *const i32;
    print_i32(nth(base, 2));
    print_i32(sum(base, unsafe { offset(base, 4isize) }));
}
