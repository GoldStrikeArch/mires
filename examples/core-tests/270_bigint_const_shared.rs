//! A 64-bit constant used by two functions and by static initializers, split across chunks.
//!
//! On `js-ecma-target=2018` there is no BigInt literal, and the JIR lane names each distinct 64-bit
//! constant of a function by one module-level binding (`jsc-link/src/lower/bigint.rs`) instead of
//! printing a `BigInt("…")` call at every use. `GOLDEN` is used by `mix`, `fold` and `only_a`, so
//! the binding must be one name across items; it is also in the initializers of `SEEDS` and
//! `PAIR`, which run once and keep the inline spelling, and the two spellings must agree
//! (`PAIR.lo == GOLDEN`). `SIGNED` pins the negative spellings, `i64::MIN` included.
//!
//! Split into two chunks (`270_bigint_const_shared.chunks`, run by `scripts/chunk-test.sh`), every
//! binding has to be owned or imported by each file that names it: `GOLDEN` and `MASK` are reached
//! by both chunks, so the shared chunk owns them; `A_ONLY` is reached by chunk `a` only and
//! `B_ONLY` by chunk `b` only. The program also runs unsplit, from `rust_entry`, which makes the
//! same calls.
//!
//! The expectations were validated by transplanting this file onto `std` and running it at `-O3`
//! and `-C opt-level=0`, which agree.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;
const MASK: u64 = 0xFFFF_FFFF_0000_0000;
const A_ONLY: u64 = 0xA5A5_A5A5_A5A5_A5A5;
const B_ONLY: u64 = 0x0123_4567_89AB_CDEF;

/// Initialized with the constants the functions use: read by chunk `a`.
static SEEDS: [u64; 3] = [GOLDEN, MASK, u64::MAX];
/// The negative spellings: read by chunk `b`.
static SIGNED: [i64; 3] = [i64::MIN, -0x1234_5678_9ABC, -1];

struct Pair {
    lo: u64,
    hi: u64,
}
/// Read by both chunks.
static PAIR: Pair = Pair { lo: GOLDEN, hi: 7 };

#[inline(never)]
fn mix(x: u64) -> u64 {
    (x ^ GOLDEN).wrapping_mul(GOLDEN)
}

#[inline(never)]
fn fold(x: u64) -> u64 {
    (x & MASK) | (x.wrapping_add(GOLDEN) & !MASK)
}

#[inline(never)]
fn only_a(x: u64) -> u64 {
    x.wrapping_mul(A_ONLY).wrapping_add(GOLDEN)
}

#[inline(never)]
fn only_b(x: i64) -> i64 {
    x.wrapping_mul(-0x1234_5678_9ABC).wrapping_sub(B_ONLY as i64)
}

#[unsafe(no_mangle)]
pub fn chunk_a() {
    print_str("chunk a");
    print_u64(mix(1));
    print_u64(mix(SEEDS[0]));
    print_u64(fold(SEEDS[1] ^ SEEDS[2]));
    print_u64(only_a(3));
    print_u64(PAIR.lo.wrapping_add(PAIR.hi));
}

#[unsafe(no_mangle)]
pub fn chunk_b() {
    print_str("chunk b");
    print_u64(fold(2));
    print_u64(mix(PAIR.hi));
    print_i64(SIGNED[0]);
    print_i64(SIGNED[1].wrapping_mul(3));
    print_i64(only_b(SIGNED[2]));
    print_bool(PAIR.lo == GOLDEN);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    chunk_a();
    chunk_b();
}
