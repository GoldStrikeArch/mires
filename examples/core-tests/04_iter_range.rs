//! `core::iter` over `Range`, which is the pointer-free half of the iterator library.
//!
//! `Range<T>` iterates by incrementing an integer, so every adapter built on it works here.
//! `slice::iter` does not: it walks a pair of raw pointers, which has no meaning in a model where
//! a value is a JavaScript object rather than bytes at an address.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // The plain `for` loop.
    let mut total = 0i32;
    for i in 0..5 {
        total += i;
    }
    print_i32(total);

    // Inclusive, stepped and reversed ranges.
    let mut inclusive = 0i32;
    for i in 1..=4 {
        inclusive += i;
    }
    print_i32(inclusive);

    let mut stepped = 0i32;
    for i in (0..10).step_by(3) {
        stepped += i;
    }
    print_i32(stepped);

    let mut backwards = 0i32;
    for i in (0..4).rev() {
        backwards = backwards * 10 + i;
    }
    print_i32(backwards);

    // Consuming adapters.
    print_i32((1..5).sum::<i32>());
    print_i32((1..5).product::<i32>());
    print_usize((0..7).count());
    print_i32((0..9).fold(0, |acc, x| acc + x * 2));
    print_i32((3..9).max().unwrap());
    print_i32((3..9).min().unwrap());
    print_i32((0..10).last().unwrap());
    print_i32((0..10).nth(3).unwrap());

    // Lazy adapters chained together.
    print_i32((0..10).map(|x| x * x).filter(|x| x % 2 == 0).sum());
    print_i32((0..10).skip(7).sum());
    print_i32((0..10).take(3).sum());
    print_i32((0..10).take_while(|x| *x < 4).sum());
    print_i32((0..10).skip_while(|x| *x < 8).sum());
    print_i32((0..5).filter_map(|x| if x % 2 == 0 { Some(x * 10) } else { None }).sum());
    print_i32((0..4).chain(10..12).sum());

    // `zip` and `enumerate` pair two counters.
    let mut zipped = 0i32;
    for (a, b) in (0..4).zip(10..20) {
        zipped += a * b;
    }
    print_i32(zipped);

    let mut indexed = 0i32;
    for (i, v) in (5..9).enumerate() {
        indexed += (i as i32) * v;
    }
    print_i32(indexed);

    // Predicates and searching.
    print_bool((0..10).any(|x| x == 7));
    print_bool((0..10).all(|x| x < 10));
    print_i32((0..10).find(|x| x % 7 == 3).unwrap());
    print_usize((0..10).position(|x| x == 6).unwrap());

    // The iterator protocol by hand.
    let mut manual = (0..3).into_iter();
    print_i32(manual.next().unwrap());
    print_i32(manual.next().unwrap());
    print_i32(manual.next().unwrap());
    print_bool(manual.next().is_none());

    // A range of `u64`, which is a `BigInt` on the JavaScript side.
    let mut wide = 0u64;
    for i in 0u64..4 {
        wide += i * 1_000_000_007;
    }
    print_u64(wide);

    // `Range` is also a value: empty, contains, len.
    let range = 2..6;
    print_bool(range.is_empty());
    print_bool(range.contains(&4));
    print_usize(range.len());
}
