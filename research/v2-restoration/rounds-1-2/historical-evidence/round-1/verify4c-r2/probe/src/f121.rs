//! Pooled constants (`js-pool-consts`, on in the size profiles): immutable constant data that
//! only shared references reach is emitted once and named at every use.
//!
//! The positive cases are the shapes that used to be printed once per use: a `const` of reference
//! type (`ROWS`, `FILTERS`, `OPS`), read through `for`/`.iter()`/indexing in several functions and
//! two closures, and a by-value `const` array borrowed by `.iter()` (`GRID`), which rustc promotes
//! to a `&'static`. `scripts/pooled-constants-test.sh` counts the copies in the program and reads
//! the lane sidecar's `pool` block.
//!
//! The controls are the cases that must stay per use, or apart:
//!
//! * `GRID` is also copied **by value** into a `let mut` local and written there, twice; each copy
//!   starts from the constant, and the borrowed readers never see the writes;
//! * `OPS` holds function pointers that are only reachable through the table, so the pooled
//!   binding must keep them alive, and they are called through it;
//! * `FIRST` and `SECOND` are `static`s holding byte-identical interior-mutable data: two live
//!   mutable values, which nothing may merge;
//! * `BYTES_U8` and `BYTES_U32` print alike as JavaScript but are different types: never one
//!   binding, so their addresses stay distinct, as natively;
//! * `INDEX` and `FIRST_FILTER` are `static`s whose initializers point into pooled data and are
//!   evaluated when the module loads, before any function runs.
//!
//! `pooled_chunk_a`/`pooled_chunk_b` are the two entry points the test script also asks for as
//! chunks: both read `ROWS`, so its binding has to live in the shared chunk.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::cell::Cell;
use core::hint::black_box;

#[derive(Clone, Copy)]
struct Row {
    name: &'static str,
    weight: u32,
    tags: &'static [&'static str],
}

/// A `const` of reference type: every use points into one promoted, immutable allocation.
const ROWS: &[Row] = &[
    Row {
        name: "pooled-row-alpha",
        weight: 3,
        tags: &["red", "small"],
    },
    Row {
        name: "pooled-row-beta",
        weight: 5,
        tags: &["blue"],
    },
    Row {
        name: "pooled-row-gamma",
        weight: 8,
        tags: &["red", "large", "round"],
    },
    Row {
        name: "pooled-row-delta",
        weight: 13,
        tags: &[],
    },
    Row {
        name: "pooled-row-epsilon",
        weight: 21,
        tags: &["green", "red"],
    },
    Row {
        name: "pooled-row-zeta",
        weight: 34,
        tags: &["square"],
    },
    Row {
        name: "pooled-row-eta-the-longest",
        weight: 55,
        tags: &["red"],
    },
    Row {
        name: "pooled-row-theta",
        weight: 89,
        tags: &["blue", "small"],
    },
];

/// The shape of FingerprintJS's `BLOCKER_FILTERS`: names beside nested slices of selectors.
const FILTERS: &[(&str, &[&str])] = &[
    (
        "pooled-filter-one",
        &["#ad-a", ".ad-b", "[id^=\"ad\"]", "a[href*=\"/click?\"]"],
    ),
    (
        "pooled-filter-two",
        &["#banner", ".sponsored", "div[class^=\"promo\"]"],
    ),
    (
        "pooled-filter-three",
        &[".popup", "#overlay", ".modal-ad", "#cookie-wall", ".newsletter"],
    ),
    (
        "pooled-filter-four",
        &["#interstitial", ".sticky-ad", "iframe[src*=\"ads\"]"],
    ),
];

/// A by-value `const`: copying it makes a fresh array, borrowing it reads a promoted one.
const GRID: [Row; 4] = [
    Row {
        name: "pooled-grid-north-cell",
        weight: 1,
        tags: &["n"],
    },
    Row {
        name: "pooled-grid-east-cell",
        weight: 2,
        tags: &["e"],
    },
    Row {
        name: "pooled-grid-south-cell",
        weight: 4,
        tags: &["s"],
    },
    Row {
        name: "pooled-grid-west-cell",
        weight: 8,
        tags: &["w"],
    },
];

#[inline(never)]
fn double(x: u32) -> u32 {
    x * 2
}

#[inline(never)]
fn square(x: u32) -> u32 {
    x * x
}

#[inline(never)]
fn flip(x: u32) -> u32 {
    1000 - x
}

#[inline(never)]
fn halve(x: u32) -> u32 {
    x / 2
}

#[inline(never)]
fn triple(x: u32) -> u32 {
    x * 3
}

#[inline(never)]
fn increment(x: u32) -> u32 {
    x + 1
}

/// Function pointers that nothing but the table names.
const OPS: &[(&str, fn(u32) -> u32)] = &[
    ("pooled-op-double", double),
    ("pooled-op-square", square),
    ("pooled-op-flip", flip),
    ("pooled-op-halve", halve),
    ("pooled-op-triple", triple),
    ("pooled-op-increment", increment),
];

/// Byte-identical as JavaScript (`[1,2,..]`), different as Rust.
const BYTES_U8: &[u8] = &[
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48,
    49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71,
    72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94,
    95, 96,
];
const BYTES_U32: &[u32] = &[
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48,
    49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71,
    72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94,
    95, 96,
];

/// Interior-mutable storage in a `static`, twice over with the same contents.
struct Shared(Cell<[u32; 16]>);

// SAFETY: the program is single threaded.
unsafe impl Sync for Shared {}

static FIRST: Shared = Shared(Cell::new([
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
]));
static SECOND: Shared = Shared(Cell::new([
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
]));

/// Statics evaluated at module load whose values point into pooled data.
static INDEX: &[Row] = ROWS;
static FIRST_FILTER: &(&str, &[&str]) = &FILTERS[0];

#[inline(never)]
fn total_weight() -> u32 {
    let mut total = 0;
    for row in ROWS {
        total += row.weight;
    }
    total
}

#[inline(never)]
fn tag_count() -> usize {
    ROWS.iter().map(|row| row.tags.len()).sum()
}

#[inline(never)]
fn longest_name() -> &'static str {
    let mut best = "";
    for row in ROWS.iter() {
        if row.name.len() > best.len() {
            best = row.name;
        }
    }
    best
}

#[inline(never)]
fn weight_of(name: &str) -> Option<u32> {
    ROWS.iter()
        .find(|row| row.name == name)
        .map(|row| row.weight)
}

#[inline(never)]
fn selector_count() -> usize {
    let mut count = 0;
    for (_, selectors) in FILTERS {
        count += selectors.len();
    }
    count
}

#[inline(never)]
fn filter_with(selector: &str) -> &'static str {
    for (name, selectors) in FILTERS.iter() {
        if selectors.contains(&selector) {
            return name;
        }
    }
    "none"
}

#[inline(never)]
fn grid_weight() -> u32 {
    GRID.iter().map(|row| row.weight).sum()
}

#[inline(never)]
fn grid_name(index: usize) -> &'static str {
    GRID[index].name
}

/// Writes into a by-value copy. Each call starts again from the constant.
#[inline(never)]
fn copy_and_write(round: u32) -> u32 {
    let mut local = GRID;
    local[0].weight += 1000 * round;
    local[1].name = "pooled-grid-written";
    local[2].tags = &["written"];
    print_str(local[1].name);
    print_usize(local[2].tags.len() + local[3].tags.len());
    let mut total = 0;
    for row in &local {
        total += row.weight;
    }
    total
}

#[inline(never)]
fn apply_all(x: u32) {
    for (name, op) in OPS {
        print_str(name);
        print_u32(op(x));
    }
}

#[inline(never)]
fn apply_named(name: &str, x: u32) -> u32 {
    for (candidate, op) in OPS.iter() {
        if *candidate == name {
            return op(x);
        }
    }
    0
}

#[inline(never)]
fn interior_statics() {
    let mut first = FIRST.0.get();
    first[0] = 99;
    first[15] = 77;
    FIRST.0.set(first);
    print_u32(FIRST.0.get()[0]);
    print_u32(FIRST.0.get()[15]);
    print_u32(SECOND.0.get()[0]);
    print_u32(SECOND.0.get()[15]);
}

#[inline(never)]
fn static_initializers() {
    print_usize(INDEX.len());
    print_str(INDEX[2].name);
    print_str(FIRST_FILTER.0);
    print_usize(FIRST_FILTER.1.len());
}

#[inline(never)]
fn byte_tables() {
    let mut small = 0u32;
    for byte in BYTES_U8 {
        small += *byte as u32;
    }
    print_u32(small);
    print_u32(BYTES_U32.iter().sum::<u32>());
    print_u32(BYTES_U8[black_box(23)] as u32 + BYTES_U32[black_box(22)]);
    // Two allocations of different contents, so two addresses, natively and here.
    print_bool(BYTES_U8.as_ptr() as usize == BYTES_U32.as_ptr() as usize);
}

#[unsafe(no_mangle)]
pub fn pooled_chunk_a() {
    print_str("chunk a");
    print_u32(total_weight());
}

#[unsafe(no_mangle)]
pub fn pooled_chunk_b() {
    print_str("chunk b");
    print_usize(tag_count());
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    pooled_chunk_a();
    pooled_chunk_b();
    print_str(longest_name());
    match weight_of(black_box("pooled-row-gamma")) {
        Some(weight) => print_u32(weight),
        None => print_str("missing"),
    }

    // Two closures over the table.
    let weight_at = |index: usize| ROWS[index].weight;
    let tagged = |tag: &str| ROWS.iter().filter(|row| row.tags.contains(&tag)).count();
    print_u32(weight_at(black_box(4)));
    print_usize(tagged(black_box("red")));

    print_usize(selector_count());
    print_str(filter_with(black_box(".modal-ad")));

    print_u32(grid_weight());
    print_u32(copy_and_write(1));
    print_u32(copy_and_write(2));
    print_u32(grid_weight());
    print_str(grid_name(black_box(1)));
    print_usize(GRID[black_box(2)].tags.len());

    apply_all(7);
    print_u32(apply_named(black_box("pooled-op-flip"), 1));

    interior_statics();
    static_initializers();
    byte_tables();
}
