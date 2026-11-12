//! Verifier repro: a pooled binding both chunks read, and a static only one chunk reads.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[derive(Clone, Copy)]
struct Row {
    name: &'static str,
    weight: u32,
}

const TABLE: &[Row] = &[
    Row { name: "c2-row-0-with-a-longish-name-to-reach-the-threshold", weight: 10 },
    Row { name: "c2-row-1-with-a-longish-name-to-reach-the-threshold", weight: 11 },
    Row { name: "c2-row-2-with-a-longish-name-to-reach-the-threshold", weight: 12 },
    Row { name: "c2-row-3-with-a-longish-name-to-reach-the-threshold", weight: 13 },
    Row { name: "c2-row-4-with-a-longish-name-to-reach-the-threshold", weight: 14 },
];

/// Read by chunk a only; its initializer points into the table.
static INDEX: &[Row] = TABLE;

#[unsafe(no_mangle)]
pub fn chunk_a() {
    let mut total = 0;
    for row in TABLE {
        total += row.weight;
    }
    print_u32(total);
    print_usize(INDEX.len());
}

#[unsafe(no_mangle)]
pub fn chunk_b() {
    print_u32(TABLE[1].weight);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    chunk_a();
    chunk_b();
}
