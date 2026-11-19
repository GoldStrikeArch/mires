//! `vec.into_iter().map(..).collect::<Vec<_>>()` and its relatives, where `alloc` would collect in
//! place: reuse the source allocation for the result whenever the result's element fits in the
//! source's (`alloc::vec::in_place_collect`).
//!
//! Reusing it means reinterpreting a block of one element type as a block of another: the buffer
//! pointer is cast from `*mut Src` to `*mut Dst` and written through while the rest of the source
//! is still being read. A heap block here is a JavaScript array of one element representation,
//! retyped at most once (`CONTRACT.md`, "Allocation"), so that cast has no meaning unless the two
//! types happen to share one. When they did not, the reuse was a compile-time refusal (`(u32,
//! usize)` to `usize`), a run-time one (a newtype to its field) or a buffer projected into its
//! first element (a one-tuple to its field). The custom sysroot therefore never collects in place
//! (`patches/0014-no-in-place-collect.patch`); the result is collected into an allocation of its
//! own, as it is whenever the layouts do not fit.
//!
//! Covered: a smaller element, a newtype, a one-tuple, two same-sized aggregates, the same type,
//! an owning element, a partly consumed source whose remaining elements must still drop, a
//! `filter_map`, and the shape the MiniSearch port collected (four fields to a struct).

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::Cell;

struct Wrapped(u32);

struct Point {
    x: u32,
    y: u32,
}

struct Counted<'a> {
    value: u32,
    drops: &'a Cell<u32>,
}

impl Drop for Counted<'_> {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

struct Suggestion {
    terms: Vec<u16>,
    score: f64,
    groups: Vec<Vec<u16>>,
    id: u32,
}

#[inline(never)]
fn second_halves(pairs: Vec<(u32, usize)>) -> Vec<usize> {
    pairs.into_iter().map(|(_, position)| position).collect()
}

#[inline(never)]
fn unwrapped(values: Vec<Wrapped>) -> Vec<u32> {
    values.into_iter().map(|value| value.0 + 1).collect()
}

#[inline(never)]
fn singles(values: Vec<(u32,)>) -> Vec<u32> {
    values.into_iter().map(|value| value.0 + 2).collect()
}

#[inline(never)]
fn points(pairs: Vec<(u32, u32)>) -> Vec<Point> {
    pairs.into_iter().map(|(x, y)| Point { x: x + y, y }).collect()
}

#[inline(never)]
fn scaled(values: Vec<u32>) -> Vec<u32> {
    values.into_iter().map(|value| value * 10).collect()
}

#[inline(never)]
fn names(entries: Vec<(String, u32)>) -> Vec<String> {
    entries
        .into_iter()
        .map(|(mut name, count)| {
            name.push(char::from(b'0' + count as u8));
            name
        })
        .collect()
}

#[inline(never)]
fn first_two(items: Vec<Counted<'_>>) -> Vec<u32> {
    items.into_iter().take(2).map(|item| item.value).collect()
}

#[inline(never)]
fn even_sums(pairs: Vec<(u32, u32)>) -> Vec<u32> {
    pairs
        .into_iter()
        .filter_map(|(a, b)| ((a + b) % 2 == 0).then_some(a + b))
        .collect()
}

#[inline(never)]
fn suggestions(rows: Vec<(Vec<u16>, f64, Vec<Vec<u16>>, u32)>) -> Vec<Suggestion> {
    rows.into_iter()
        .map(|(terms, score, groups, id)| Suggestion {
            terms,
            score,
            groups,
            id,
        })
        .collect()
}

fn print_all(values: &[u32]) {
    print_usize(values.len());
    for value in values {
        print_u32(*value);
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let pairs: Vec<(u32, usize)> = "héllo"
        .char_indices()
        .map(|(at, c)| (c as u32, at))
        .collect();
    let positions = second_halves(pairs);
    print_usize(positions.len());
    for position in &positions {
        print_usize(*position);
    }

    print_all(&unwrapped((1..6).map(|value| Wrapped(value)).collect()));
    print_all(&singles((1..6).map(|value| (value,)).collect()));

    let points = points((1..5).map(|value| (value, value * 2)).collect());
    print_usize(points.len());
    for point in &points {
        print_u32(point.x);
        print_u32(point.y);
    }

    let mut grown = scaled(vec![1, 2, 3]);
    grown.push(40);
    print_all(&grown);

    let named = names(vec![(String::from("a"), 1), (String::from("b"), 2)]);
    for name in &named {
        print_str(name);
    }

    let drops = Cell::new(0);
    let items: Vec<Counted<'_>> = (1..4)
        .map(|value| Counted {
            value,
            drops: &drops,
        })
        .collect();
    let taken = first_two(items);
    print_all(&taken);
    print_u32(drops.get());

    print_all(&even_sums(vec![(1, 1), (1, 2), (2, 4), (3, 4)]));

    let rows = vec![
        (vec![1u16, 2], 1.5f64, vec![vec![3u16]], 7u32),
        (vec![4u16], 2.0f64, Vec::new(), 8u32),
    ];
    let found = suggestions(rows);
    print_usize(found.len());
    for suggestion in &found {
        print_usize(suggestion.terms.len());
        print_u32((suggestion.score * 2.0) as u32);
        print_usize(suggestion.groups.len());
        print_u32(suggestion.id);
    }
}
