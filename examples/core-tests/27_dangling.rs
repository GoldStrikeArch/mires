//! The pointer a collection holds before it has ever allocated.
//!
//! `Vec::new()` does not allocate: its data pointer is `NonNull::dangling()`, which this model
//! holds as a bare number (the alignment). Everything that only ever compares such a pointer with
//! null worked; everything that builds a *slice* out of it did not. `&v[..]` is
//! `{ buf: p.buf, off: p.off, len: 0 }` over a number, which is `{ buf: undefined, off: undefined }`,
//! and `slice::Iter::new` then offsets that by the length -- `ptr.add(0)`, which is sound Rust on a
//! dangling pointer -- and produces the `{ buf: undefined, off: NaN }` record `__rt._named` refuses.
//!
//! So `for x in &v` over a `Vec` that has never been pushed to THREW, and this fixture is why it
//! does not any more: `__rt.unscale` now hands a dangling address a frozen empty buffer of its own
//! rather than passing the number through. Two dangling pointers of one alignment are then equal,
//! which is exactly what makes `iter.ptr === iter.end` end the loop on its first test.
//!
//! Every shape below is sound Rust that a real program writes. `let mut rows = Vec::new();`
//! followed by a `for` over it before anything is pushed is what an empty feed renders.
//!
//! What is deliberately NOT here: offsetting a genuine integer address, which Rust does forbid and
//! `__rt._named` still refuses. That refusal is the reason this file has to distinguish the two
//! cases rather than making the record shape legal everywhere.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::string::String;
use alloc::vec::Vec;

/// The failing shape, exactly: a `for` over a borrow of a never-allocated `Vec`.
fn count(values: &Vec<i32>) -> i32 {
    let mut n = 0;
    for _ in values.iter() {
        n += 1;
    }
    n
}

/// The same loop over a slice, so the fat pointer is built by the caller rather than by `deref`.
fn total(values: &[i32]) -> i32 {
    let mut sum = 0;
    for value in values {
        sum += *value;
    }
    sum
}

/// An aggregate element, whose block retypes to objects rather than numbers. Its dangling pointer
/// is a different alignment from `i32`'s, which is the case a single shared empty buffer would get
/// wrong.
#[derive(Clone, Copy)]
struct Point {
    x: i32,
    y: i32,
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. The bug. Never pushed, never allocated.
    let never: Vec<i32> = Vec::new();
    print_i32(count(&never));
    print_i32(total(&never));
    print_bool(never.is_empty());
    print_usize(never.len());

    // 2. `with_capacity(0)` is dangling too, and reaches the same pointer by another route.
    let sized: Vec<i32> = Vec::with_capacity(0);
    print_i32(count(&sized));

    // 3. Two dangling pointers of one alignment must compare EQUAL, or the iterator never
    //    terminates. Asked directly rather than only through a loop.
    let a: Vec<i32> = Vec::new();
    let b: Vec<i32> = Vec::new();
    print_bool(a.as_ptr() == b.as_ptr());
    print_bool(!a.as_ptr().is_null());

    // 4. A wider alignment, so the buffer is a different one.
    let wide: Vec<i64> = Vec::new();
    let mut w = 0;
    for _ in wide.iter() {
        w += 1;
    }
    print_i32(w);

    // 5. An aggregate element.
    let points: Vec<Point> = Vec::new();
    let mut px = 0;
    for point in points.iter() {
        px += point.x + point.y;
    }
    print_i32(px);

    // 6. A never-allocated `String`, which is a `Vec<u8>` under a different name, iterated by
    //    chars and by bytes.
    let empty = String::new();
    print_usize(empty.len());
    print_i32(empty.chars().count() as i32);
    print_i32(empty.as_bytes().iter().count() as i32);

    // 7. The pointer is still usable AFTER the vector allocates, which is what proves the empty
    //    buffer is a starting state and not a permanent one.
    let mut grown: Vec<i32> = Vec::new();
    print_i32(count(&grown));
    grown.push(7);
    grown.push(11);
    print_i32(count(&grown));
    print_i32(total(&grown));

    // 8. And back to empty, which is the ALLOCATED-empty case that always worked. It must keep
    //    working, and it must not be confused with the dangling one: this pointer is a real
    //    buffer, so it is NOT equal to a fresh `Vec`'s.
    grown.clear();
    print_i32(count(&grown));
    print_bool(grown.as_ptr() == never.as_ptr());

    print_str("dangling ok");
}
