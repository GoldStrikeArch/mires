//! `Vec`: the allocation the heap model was built for.
//!
//! A `Vec` is a `RawVec`, which is a byte granular pointer plus a capacity, and every access casts
//! that pointer back to `*mut T`. So this exercises the retype directly (`CONTRACT.md`,
//! "Allocation"): growth through `push`, the in-place `realloc`, the zeroed block `vec![0; n]`
//! asks for, and the borrow of the whole thing as an ordinary `&[T]`.
//!
//! Both a scalar element and an aggregate one, because they retype to different things: a block of
//! `i32` holds numbers, a block of `Point` holds objects, and only the second needs a fresh object
//! per element when the block was zeroed.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::vec;
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq)]
struct Point {
    x: i32,
    y: i32,
}

fn sum(values: &[i32]) -> i32 {
    let mut total = 0;
    for value in values {
        total += *value;
    }
    total
}

fn largest(values: &[i32]) -> i32 {
    let mut best = values[0];
    for value in values {
        if *value > best {
            best = *value;
        }
    }
    best
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Growth: every `push` past the capacity is a `realloc` of the same block.
    let mut numbers: Vec<i32> = Vec::new();
    let mut i = 0;
    while i < 10 {
        numbers.push(i * i);
        i += 1;
    }
    print_usize(numbers.len());
    print_i32(numbers[0]);
    print_i32(numbers[9]);

    // Reading back: index, iterate, and borrow into a function that takes a plain slice.
    print_i32(sum(&numbers));
    print_i32(largest(&numbers));
    let mut iterated = 0;
    for value in &numbers {
        iterated += *value;
    }
    print_i32(iterated);

    // Shrinking, and the tail.
    print_i32(numbers.pop().unwrap());
    print_usize(numbers.len());

    // In the middle.
    numbers.insert(0, 100);
    print_i32(numbers[0]);
    print_i32(numbers.remove(0));
    print_i32(numbers[0]);

    // Sorting, which reorders the block in place through raw pointers.
    let mut unsorted: Vec<i32> = Vec::new();
    unsorted.push(5);
    unsorted.push(1);
    unsorted.push(4);
    unsorted.push(2);
    unsorted.push(3);
    unsorted.sort();
    print_i32(unsorted[0]);
    print_i32(unsorted[4]);

    // A reserved block, filled afterwards, and extended from a slice.
    let mut reserved: Vec<i32> = Vec::with_capacity(16);
    print_usize(reserved.len());
    reserved.extend_from_slice(&[7, 8, 9]);
    print_usize(reserved.len());
    print_i32(sum(&reserved));

    // A zeroed block of scalars: `vec![0; n]` is `alloc_zeroed`, and the retype has to fill it.
    let zeros: Vec<i32> = vec![0; 5];
    print_usize(zeros.len());
    print_i32(sum(&zeros));

    // The same for a `bool`, whose zero is `false` rather than the number 0.
    let flags: Vec<bool> = vec![false; 4];
    print_usize(flags.len());
    print_bool(flags[0]);
    print_bool(flags[3]);

    // A repeated non-zero element takes the ordinary path.
    let ones: Vec<i32> = vec![1; 3];
    print_i32(sum(&ones));

    // An aggregate element: the block holds objects, and each has to be its own.
    let mut points: Vec<Point> = Vec::new();
    points.push(Point { x: 1, y: 2 });
    points.push(Point { x: 3, y: 4 });
    points.push(Point { x: 5, y: 6 });
    print_usize(points.len());
    print_i32(points[1].x + points[1].y);

    points[0].x = 50;
    print_i32(points[0].x);
    print_i32(points[1].x);

    let mut total = 0;
    for point in &points {
        total += point.x * point.y;
    }
    print_i32(total);

    points.sort_by(|a, b| b.x.cmp(&a.x));
    print_i32(points[0].x);
    print_i32(points[2].x);

    let popped = points.pop().unwrap();
    print_i32(popped.x + popped.y);
    print_usize(points.len());

    // A zeroed block of aggregates: every element is a distinct object, so a write through one
    // must not be visible through the next.
    let mut origins: Vec<Point> = vec![Point { x: 0, y: 0 }; 3];
    origins[0].x = 9;
    print_i32(origins[0].x);
    print_i32(origins[1].x);
    print_bool(origins[1] == origins[2]);
}
