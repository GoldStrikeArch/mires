//! Cross-chunk ESM verification for `$__mires_i64_*` helper static & writer co-location.
//!
//! `chunk_a` uses only 64-bit multiplication (`$__mires_i64_mul` / `$__mires_i64_mul_ovf`), while
//! `chunk_b` uses only 64-bit division (`$__mires_i64_div`) and `f64::to_bits`
//! (`$__mires_i64_f64_to_bits`). All write `$__mires_i64_hi`, which must stay in the same ESM
//! module as every active writer so importing it never throws `TypeError: Assignment to constant
//! variable`.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[inline(never)]
fn chunk_a_step(x: u64, y: u64) -> (u64, bool) {
    let p = x.wrapping_mul(y);
    let (q, ovf) = x.overflowing_mul(y);
    (p ^ q.rotate_left(7), ovf)
}

#[inline(never)]
fn chunk_b_step(x: u64, y: u64, f: f64) -> u64 {
    let d = x.wrapping_div(y | 1);
    let r = (x as i64).wrapping_rem((y as i64) | 1) as u64;
    d.wrapping_add(r) ^ f.to_bits()
}

#[unsafe(no_mangle)]
pub fn chunk_a() {
    let (v1, o1) = chunk_a_step(0x1234_5678_9abc_def1u64, 0x9e37_79b9_7f4a_7c15u64);
    print_u64(v1);
    print_bool(o1);
    let (v2, o2) = chunk_a_step(123_456_789u64, 7u64);
    print_u64(v2);
    print_bool(o2);
}

#[unsafe(no_mangle)]
pub fn chunk_b() {
    let v1 = chunk_b_step(0xfedc_ba98_7654_3210u64, 0x1000_0000_0000_0003u64, -3.5f64);
    print_u64(v1);
    let v2 = chunk_b_step(987_654_321_098_765u64, 123_456u64, 42.25f64);
    print_u64(v2);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    chunk_a();
    chunk_b();
}
