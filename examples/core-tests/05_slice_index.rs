//! Arrays and slices: coercion, length, indexing, the `SliceIndex` accessors, slice patterns.
//!
//! Everything here is index based, and that is the whole boundary of this milestone. `&s[a..b]`,
//! `split_at`, `swap`, `contains` and `s.iter()` are all *out*: each of them is implemented in
//! `core` as raw pointer arithmetic (`slice.as_ptr().add(n)` fed back through
//! `slice_from_raw_parts`), and a thin `*const T` in this backend is a JavaScript value with no
//! backing array and no offset, so a new slice cannot be built out of one. Indexing, on the other
//! hand, is the `slice_get_unchecked` intrinsic, which the backend lowers natively.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

fn sum(values: &[i32]) -> i32 {
    let mut total = 0;
    let mut i = 0;
    while i < values.len() {
        total += values[i];
        i += 1;
    }
    total
}

fn describe(values: &[i32]) -> &'static str {
    match values {
        [] => "empty",
        [_] => "one",
        [first, .., last] if first == last => "bookends",
        [_, ..] => "many",
    }
}

fn scale(values: &mut [i32], factor: i32) {
    let mut i = 0;
    while i < values.len() {
        values[i] *= factor;
        i += 1;
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let array = [10i32, 20, 30, 40, 50];

    // An array coerces to a slice, and the slice knows its length.
    print_i32(sum(&array));
    print_usize(array.len());
    print_bool(array.is_empty());
    print_bool(<[i32]>::is_empty(&[]));

    // Indexing, constant and computed.
    print_i32(array[0]);
    print_i32(array[4]);
    let i = 2usize;
    print_i32(array[i]);

    // The non-panicking accessors.
    print_i32(*array.get(1).unwrap());
    print_bool(array.get(9).is_none());
    print_i32(*array.first().unwrap());
    print_i32(*array.last().unwrap());
    print_bool(<[i32]>::first(&[]).is_none());

    // Mutation through `&mut [T]`.
    let mut buffer = [1i32, 2, 3, 4];
    buffer[0] = 100;
    scale(&mut buffer, 3);
    print_i32(sum(&buffer));
    if let Some(slot) = buffer.get_mut(3) {
        *slot = -1;
    }
    print_i32(buffer[3]);
    *buffer.last_mut().unwrap() = 7;
    print_i32(buffer[3]);

    // Slice patterns, including one that binds through a `..`.
    print_str(describe(&[]));
    print_str(describe(&[7]));
    print_str(describe(&[7, 8, 7]));
    print_str(describe(&[7, 8, 9]));
    if let [a, b, rest @ ..] = &array[..] {
        print_i32(*a + *b);
        print_usize(rest.len());
    }

    // A slice of an aggregate element.
    #[derive(Clone, Copy)]
    struct Point {
        x: i32,
        y: i32,
    }
    let points = [Point { x: 1, y: 2 }, Point { x: 3, y: 4 }];
    print_i32(points[0].x + points[1].y);
    print_usize(points.len());

    // A two dimensional array, indexed twice.
    let grid = [[1i32, 2, 3], [4, 5, 6]];
    print_i32(grid[1][2]);
    print_usize(grid.len());
    print_usize(grid[0].len());
}
