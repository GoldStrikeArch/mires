
#![no_std]
#![no_main]
#![allow(unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::cell::Cell;
use core::hint::black_box;

#[derive(Clone, Copy)]
struct Row { name: &'static str, weight: u32, arr: [u32; 3] }
const T_AB: &[Row] = &[
    Row { name: "c1-ab-row-0-with-a-longish-name", weight: 10, arr: [300, 400, 500] },
    Row { name: "c1-ab-row-1-with-a-longish-name", weight: 11, arr: [301, 401, 501] },
    Row { name: "c1-ab-row-2-with-a-longish-name", weight: 12, arr: [302, 402, 502] },
    Row { name: "c1-ab-row-3-with-a-longish-name", weight: 13, arr: [303, 403, 503] },
    Row { name: "c1-ab-row-4-with-a-longish-name", weight: 14, arr: [304, 404, 504] },
    Row { name: "c1-ab-row-5-with-a-longish-name", weight: 15, arr: [305, 405, 505] },
    Row { name: "c1-ab-row-6-with-a-longish-name", weight: 16, arr: [306, 406, 506] },
    Row { name: "c1-ab-row-7-with-a-longish-name", weight: 17, arr: [307, 407, 507] }
];
const T_A: &[Row] = &[
    Row { name: "c1-a-row-0-with-a-longish-name", weight: 10, arr: [300, 400, 500] },
    Row { name: "c1-a-row-1-with-a-longish-name", weight: 11, arr: [301, 401, 501] },
    Row { name: "c1-a-row-2-with-a-longish-name", weight: 12, arr: [302, 402, 502] },
    Row { name: "c1-a-row-3-with-a-longish-name", weight: 13, arr: [303, 403, 503] },
    Row { name: "c1-a-row-4-with-a-longish-name", weight: 14, arr: [304, 404, 504] },
    Row { name: "c1-a-row-5-with-a-longish-name", weight: 15, arr: [305, 405, 505] },
    Row { name: "c1-a-row-6-with-a-longish-name", weight: 16, arr: [306, 406, 506] },
    Row { name: "c1-a-row-7-with-a-longish-name", weight: 17, arr: [307, 407, 507] }
];
const T_B: &[Row] = &[
    Row { name: "c1-b-row-0-with-a-longish-name", weight: 10, arr: [300, 400, 500] },
    Row { name: "c1-b-row-1-with-a-longish-name", weight: 11, arr: [301, 401, 501] },
    Row { name: "c1-b-row-2-with-a-longish-name", weight: 12, arr: [302, 402, 502] },
    Row { name: "c1-b-row-3-with-a-longish-name", weight: 13, arr: [303, 403, 503] },
    Row { name: "c1-b-row-4-with-a-longish-name", weight: 14, arr: [304, 404, 504] },
    Row { name: "c1-b-row-5-with-a-longish-name", weight: 15, arr: [305, 405, 505] },
    Row { name: "c1-b-row-6-with-a-longish-name", weight: 16, arr: [306, 406, 506] },
    Row { name: "c1-b-row-7-with-a-longish-name", weight: 17, arr: [307, 407, 507] }
];
const T_C: &[Row] = &[
    Row { name: "c1-c-row-0-with-a-longish-name", weight: 10, arr: [300, 400, 500] },
    Row { name: "c1-c-row-1-with-a-longish-name", weight: 11, arr: [301, 401, 501] },
    Row { name: "c1-c-row-2-with-a-longish-name", weight: 12, arr: [302, 402, 502] },
    Row { name: "c1-c-row-3-with-a-longish-name", weight: 13, arr: [303, 403, 503] },
    Row { name: "c1-c-row-4-with-a-longish-name", weight: 14, arr: [304, 404, 504] },
    Row { name: "c1-c-row-5-with-a-longish-name", weight: 15, arr: [305, 405, 505] },
    Row { name: "c1-c-row-6-with-a-longish-name", weight: 16, arr: [306, 406, 506] },
    Row { name: "c1-c-row-7-with-a-longish-name", weight: 17, arr: [307, 407, 507] }
];
#[inline(never)] fn dbl(x: u32) -> u32 { x * 2 }
#[inline(never)] fn sq(x: u32) -> u32 { x * x }
#[inline(never)] fn neg(x: u32) -> u32 { 1000 - x }
#[inline(never)] fn inc(x: u32) -> u32 { x + T_C[1].weight }
const OPS_AB: &[(&str, fn(u32) -> u32)] = &[
    ("c1-op-double-with-a-long-name", dbl), ("c1-op-square-with-a-long-name", sq), ("c1-op-negate-with-a-long-name", neg), ("c1-op-increment-with-long-name", inc),
    ("c1-op-double-again-long-name", dbl), ("c1-op-square-again-long-name", sq), ("c1-op-negate-again-long-name", neg), ("c1-op-increment-again-long-name", inc),
];
// A static only chunk a reads, initialized from the table both chunks read.
static IDX_A: &[Row] = T_AB;
// A static only chunk b reads, initialized from a table only chunk a reads otherwise.
static IDX_B: (&[Row], &[Row]) = (T_A, T_B);
// An interior-mutable static both chunks touch, initialized from pooled data.
struct Shared(Cell<&'static [Row]>);
unsafe impl Sync for Shared {}
static BOTH: Shared = Shared(Cell::new(T_AB));
static ONLY_C: &[(&str, fn(u32) -> u32)] = OPS_AB;

#[inline(never)] fn sum(rows: &[Row]) -> u32 { rows.iter().map(|r| r.weight + r.arr[0]).sum() }

#[unsafe(no_mangle)]
pub fn chunk_a(depth: u32) {
    print_str("chunk a");
    print_u32(sum(T_AB)); print_u32(sum(T_A)); print_u32(T_A[3].arr[2]); print_u32(sum(IDX_A));
    for (name, op) in OPS_AB { print_u32(op(7)); }
    print_u32(sum(BOTH.0.get())); BOTH.0.set(&T_A[1..]);
    if depth > 0 { chunk_b(depth - 1); }
}
#[unsafe(no_mangle)]
pub fn chunk_b(depth: u32) {
    print_str("chunk b");
    print_u32(sum(T_AB)); print_u32(sum(T_B)); print_u32(T_B[2].arr[1]); print_u32(sum(IDX_B.0) + sum(IDX_B.1));
    print_u32((OPS_AB[black_box(1)].1)(9)); print_str(OPS_AB[2].0);
    print_u32(sum(BOTH.0.get())); BOTH.0.set(&T_B[2..]);
    if depth > 0 { chunk_a(depth - 1); }
}
#[unsafe(no_mangle)]
pub fn chunk_c() {
    print_str("chunk c");
    print_u32(sum(T_C)); print_u32(T_C[7].weight); print_u32((ONLY_C[black_box(3)].1)(1)); print_u32(sum(T_C));
}
#[unsafe(no_mangle)]
pub fn rust_entry() { chunk_a(2); chunk_b(1); chunk_c(); }
