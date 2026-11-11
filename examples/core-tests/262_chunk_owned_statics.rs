//! A chunk declares the statics it owns and imports the rest: never both.
//!
//! Split into three chunks (`262_chunk_owned_statics.chunks`, run by `scripts/chunk-test.sh`),
//! every static here is owned by exactly one file. `BASE` and `COUNTER` are read by chunks `a`
//! and `b`, so the shared chunk owns them; `INDEX_A`, `A_COUNTER` and `A_NEXT` are read by chunk
//! `a` only and `INDEX_B` and `B_COUNTERS` by chunk `b` only, and each of those names a static or
//! an entry point another file owns. The per-chunk print order used to follow such a name into
//! the chunk that imports it and declare the binding a second time, which is a `SyntaxError`
//! before the module runs. The program also runs unsplit, from `rust_entry`, which makes the same
//! calls.
//!
//! Pinned: a chunk-owned static naming a shared static, from each of two chunks (`INDEX_A`,
//! `INDEX_B`); a chunk-owned static naming another chunk's entry point (`A_NEXT -> chunk_c`, so
//! `a.js` imports from `c.js`); an interior-mutable shared static written from two chunks through
//! chunk-owned references; `ptr::eq` across files. A chunk whose entry point another chunk names
//! owns nothing but that entry, because everything the entry reaches is then reached twice.
//!
//! The expectations were validated by transplanting this file onto `std` and running it at `-O3`
//! and `-C opt-level=0`, which agree.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::cell::Cell;

/// Read by chunks a and b: owned by the shared chunk.
static BASE: [u32; 4] = [10, 11, 12, 13];
/// Read by chunk a only; points at the shared static.
static INDEX_A: &[u32; 4] = &BASE;
/// Read by chunk b only; points at the shared static.
static INDEX_B: (&[u32; 4], u32) = (&BASE, 7);

struct Shared(Cell<u32>);
unsafe impl Sync for Shared {}
/// Interior mutable, chunks a and b write it.
static COUNTER: Shared = Shared(Cell::new(100));
/// Chunk a only; refers to the interior-mutable shared static.
static A_COUNTER: &Shared = &COUNTER;
/// Chunk b only; refers to the shared static through an array of references.
static B_COUNTERS: [&Shared; 2] = [&COUNTER, &COUNTER];

/// Chunk a owns this static; it names chunk c's entry point, which chunk c owns.
static A_NEXT: fn() = chunk_c;
/// Read by chunk c, and so by chunk a through `A_NEXT`: shared.
static C_TABLE: [u32; 2] = [3, 4];

#[inline(never)]
fn shared_sum(rows: &[u32]) -> u32 {
    rows.iter().sum()
}

#[unsafe(no_mangle)]
pub fn chunk_a() {
    print_str("chunk a");
    print_u32(shared_sum(&BASE));
    print_u32(shared_sum(INDEX_A));
    print_usize(INDEX_A.len());
    print_bool(core::ptr::eq(INDEX_A, &BASE));
    A_COUNTER.0.set(A_COUNTER.0.get() + 1);
    print_u32(COUNTER.0.get());
    A_NEXT();
}

#[unsafe(no_mangle)]
pub fn chunk_b() {
    print_str("chunk b");
    print_u32(BASE[1]);
    print_u32(shared_sum(INDEX_B.0) + INDEX_B.1);
    print_usize(INDEX_B.0.len());
    print_bool(core::ptr::eq(INDEX_B.0, &BASE));
    B_COUNTERS[1].0.set(B_COUNTERS[0].0.get() * 2);
    print_u32(COUNTER.0.get());
    print_bool(core::ptr::eq(B_COUNTERS[0], &COUNTER));
}

#[unsafe(no_mangle)]
pub fn chunk_c() {
    print_str("chunk c");
    print_u32(C_TABLE[0] * 10 + C_TABLE[1]);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    chunk_a();
    chunk_b();
    chunk_c();
}
