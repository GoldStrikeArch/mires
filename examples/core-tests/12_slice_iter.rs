//! `slice::iter()` and the adapters on it.
//!
//! This is the test the pointer model exists for. `[T]::iter` is
//! `Iter { ptr: NonNull::from(slice).cast(), end_or_len: ptr.add(len) }` — a pair of raw pointers
//! into the slice's buffer, walked by `add`, compared by address, subtracted with `offset_from`
//! and dereferenced with `read`. In stage 1 a `*const T` was the pointee's JavaScript value, with
//! no buffer and no offset, so none of that could be expressed and this whole file was out. With a
//! pointer as a slot — a buffer plus an offset in elements — every one of those operations has an
//! exact JavaScript form, and `Iter` is an ordinary struct again.
//!
//! Two element types are here for a reason beyond coverage:
//!
//! * a struct element, because a raw pointer to an aggregate is a *slot* rather than the object
//!   (`{ buf: points, off: 2 }`), which is what lets `ptr.add(1)` reach the next element;
//! * a zero sized element, because `Iter` does not hold a pointer at all in that case: `end_or_len`
//!   is the remaining *count*, produced by `wrapping_byte_add` on a pointer that is never
//!   dereferenced. That is the arm where a pointer degrades to a provenance free number, and it
//!   still has to count correctly.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[derive(Clone, Copy)]
struct Point {
    x: i32,
    y: i32,
}

/// A zero sized element: `size_of::<Marker>() == 0`.
#[derive(Clone, Copy)]
struct Marker;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let values = [10i32, 20, 30, 40, 50];

    // `iter()`, by hand and through a `for`.
    let mut it = values.iter();
    print_i32(*it.next().unwrap());
    print_i32(*it.next().unwrap());

    let mut total = 0;
    for value in values.iter() {
        total += *value;
    }
    print_i32(total);

    // `for x in &xs` is the same iterator through `IntoIterator for &[T; N]`.
    let mut again = 0;
    for value in &values {
        again += *value;
    }
    print_i32(again);
    print_bool(total == again);

    // `map().sum()`, the adapter chain over a slice rather than a range.
    print_i32(values.iter().map(|v| *v * 2).sum::<i32>());
    print_i32(values.iter().copied().filter(|v| *v > 25).sum::<i32>());
    print_usize(values.iter().count());

    // `rev` walks the same buffer from the other end.
    let mut digits = 0;
    for value in values.iter().rev() {
        digits = digits * 10 + *value / 10;
    }
    print_i32(digits);
    print_i32(*values.iter().rev().next().unwrap());

    // `nth`, `position`, `find`.
    print_i32(*values.iter().nth(2).unwrap());
    print_bool(values.iter().nth(5).is_none());
    print_usize(values.iter().position(|v| *v == 40).unwrap());
    print_bool(values.iter().position(|v| *v == 41).is_none());
    print_i32(*values.iter().find(|v| **v > 25).unwrap());
    print_bool(values.iter().find(|v| **v > 100).is_none());
    print_bool(values.iter().any(|v| *v == 30));
    print_bool(values.iter().all(|v| *v > 5));

    // `enumerate` and `zip`.
    let mut weighted = 0;
    for (index, value) in values.iter().enumerate() {
        weighted += (index as i32) * *value;
    }
    print_i32(weighted);

    let factors = [1i32, 2, 3];
    let mut dot = 0;
    for (a, b) in values.iter().zip(factors.iter()) {
        dot += *a * *b;
    }
    print_i32(dot);
    print_usize(values.iter().zip(factors.iter()).count());

    // `last`, `min`, `max`, `fold`.
    print_i32(*values.iter().last().unwrap());
    print_i32(*values.iter().min().unwrap());
    print_i32(*values.iter().max().unwrap());
    print_i32(values.iter().fold(1, |acc, v| acc + *v / 10));

    // `len`, `is_empty` and `as_slice` on a partly consumed iterator: the length is
    // `end.offset_from(ptr)`, which is exactly the difference of two slot offsets.
    let mut partial = values.iter();
    partial.next();
    partial.next();
    print_usize(partial.len());
    print_bool(partial.as_slice().is_empty());
    let rest = partial.as_slice();
    print_usize(rest.len());
    print_i32(rest[0]);
    print_i32(rest[2]);
    print_i32(partial.as_slice().iter().sum::<i32>());
    while partial.next().is_some() {}
    print_usize(partial.len());
    print_bool(partial.as_slice().is_empty());
    print_bool(values.iter().as_slice().len() == 5);

    // `iter_mut`: the same walk, writing through the slot instead of reading.
    let mut buffer = [1i32, 2, 3, 4];
    for slot in buffer.iter_mut() {
        *slot *= 10;
    }
    print_i32(buffer[0]);
    print_i32(buffer[3]);
    for slot in &mut buffer {
        *slot += 1;
    }
    print_i32(buffer[1]);
    if let Some(first) = buffer.iter_mut().next() {
        *first = -5;
    }
    print_i32(buffer[0]);
    print_i32(buffer.iter().sum::<i32>());
    if let Some(third) = buffer.iter_mut().nth(2) {
        *third = 0;
    }
    print_i32(buffer[2]);

    // A slice of an aggregate element. A `*const Point` is a slot naming the array place, so
    // `add(1)` reaches the next element rather than the next field.
    let points = [
        Point { x: 1, y: 2 },
        Point { x: 3, y: 4 },
        Point { x: 5, y: 6 },
    ];
    let mut sum_x = 0;
    let mut sum_y = 0;
    for point in points.iter() {
        sum_x += point.x;
        sum_y += point.y;
    }
    print_i32(sum_x);
    print_i32(sum_y);
    print_i32(points.iter().map(|p| p.x * p.y).sum::<i32>());
    print_i32(points.iter().rev().nth(0).unwrap().x);
    print_i32(points.iter().nth(1).unwrap().y);
    print_usize(points.iter().position(|p| p.x == 5).unwrap());

    let mut movable = [Point { x: 1, y: 1 }, Point { x: 2, y: 2 }];
    for point in movable.iter_mut() {
        point.x += 10;
    }
    print_i32(movable[0].x + movable[1].x);

    // A slice of a zero sized element: `Iter` counts instead of pointing.
    let markers = [Marker, Marker, Marker];
    print_usize(markers.len());
    print_usize(markers.iter().count());
    let mut seen = 0;
    for _marker in markers.iter() {
        seen += 1;
    }
    print_i32(seen);
    print_bool(markers.iter().nth(2).is_some());
    print_bool(markers.iter().nth(3).is_none());
    let mut zst_iter = markers.iter();
    zst_iter.next();
    print_usize(zst_iter.len());
    print_usize(zst_iter.as_slice().len());
    print_bool(<[Marker]>::is_empty(&[]));
    print_usize(<[Marker]>::iter(&[]).count());

    // `split_at`, which rebuilds two slices out of one buffer and an offset.
    let (head, tail) = values.split_at(2);
    print_usize(head.len());
    print_usize(tail.len());
    print_i32(head[1]);
    print_i32(tail[0]);
    print_i32(tail.iter().sum::<i32>());
    let (empty, whole) = values.split_at(0);
    print_bool(empty.is_empty());
    print_usize(whole.len());
    let (all, none) = values.split_at(5);
    print_usize(all.len());
    print_bool(none.is_empty());

    // A subslice is a view of the same buffer, and iterating it walks that buffer.
    let middle = &values[1..4];
    print_usize(middle.len());
    print_i32(middle[0]);
    print_i32(middle.iter().sum::<i32>());
    print_i32(*middle.iter().last().unwrap());
    let (left, right) = middle.split_at(1);
    print_i32(left[0]);
    print_i32(right.iter().sum::<i32>());
}
