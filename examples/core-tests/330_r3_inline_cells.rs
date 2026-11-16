//! End-to-end fixture for Slice R3-C:
//! 1. Boxed-cell demotion (`Ord::max`/`Ord::min`, mutated `&mut i32` local & param cells).
//! 2. Used-`#[track_caller]` inlining (`slice.chunks_exact(4)`).
//! 3. Trivial 1-op wrapper inlining (`u64::wrapping_mul`) & multi-site `SoleCaller` batching (`i32::wrapping_add`).

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[inline(never)]
fn max_pair(a: u32, b: u32) -> u32 {
    core::cmp::Ord::max(a, b)
}

#[inline(never)]
fn min_pair(a: i32, b: i32) -> i32 {
    core::cmp::Ord::min(a, b)
}

#[inline(never)]
fn bump_param(mut a: i32) -> i32 {
    let r = &mut a;
    *r = (*r).wrapping_add(7);
    a
}

#[inline(never)]
fn step_adds(a: i32, b: i32, c: i32, d: i32) -> i32 {
    a.wrapping_add(b)
        .wrapping_add(c)
        .wrapping_add(d)
        .wrapping_add(11)
        .wrapping_add(13)
        .wrapping_add(17)
}

#[inline(never)]
fn murmur_chunks(bytes: &[u8]) -> u64 {
    let mut acc: u64 = 1;
    for chunk in bytes.chunks_exact(4) {
        let w = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as u64;
        acc = acc.wrapping_mul(5).wrapping_add(w);
    }
    acc
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(max_pair(17, 42));
    print_i32(min_pair(-5, 9));
    print_i32(bump_param(35));
    print_i32(step_adds(1, 2, 3, 4));
    let data = [1u8, 2, 3, 4, 10, 20, 30, 40, 99];
    print_u64(murmur_chunks(&data));
}
