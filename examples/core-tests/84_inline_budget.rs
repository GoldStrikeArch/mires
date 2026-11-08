//! The inline budget refused (Phase 5, D6): a single-caller callee whose splice would grow the
//! program past +0.5% declines with `cost.budget` and the bytes stay exactly as they were.
//!
//! `churn` is the only call `rust_entry` makes of it, so the single-caller criterion offers the
//! site to the inliner; the body is a long straight line of data-dependent arithmetic no fold
//! shortens, so the measured growth blows both the +0.5% program ceiling and the caller's 2x
//! limit and the site is reverted whole — the sidecar's `inline.declined_budget` counts it,
//! `applied` does not, and no partial splice survives.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[inline(never)]
fn churn(seed: u32) -> u32 {
    let mut a = seed;
    let mut b = seed ^ 0x9e37_79b9;
    a = a.wrapping_mul(0x0001_0dcd).rotate_left(3) ^ b;
    b = b.wrapping_add(a).rotate_left(7) ^ 0x85eb_ca6b;
    a = a.wrapping_mul(0xc2b2_ae35).rotate_left(11) ^ b;
    b = b.wrapping_add(a).rotate_left(13) ^ 0x27d4_eb2f;
    a = a.wrapping_mul(0x1656_67b1).rotate_left(17) ^ b;
    b = b.wrapping_add(a).rotate_left(19) ^ 0x1b87_3593;
    a = a.wrapping_mul(0x0000_85eb).rotate_left(23) ^ b;
    b = b.wrapping_add(a).rotate_left(29) ^ 0xcc9e_2d51;
    a = a.wrapping_mul(0x0000_ca6b).rotate_left(2) ^ b;
    b = b.wrapping_add(a).rotate_left(5) ^ 0x52dc_e729;
    a = a.wrapping_mul(0x0001_2543).rotate_left(6) ^ b;
    b = b.wrapping_add(a).rotate_left(9) ^ 0x3856_6cf9;
    a = a.wrapping_mul(0x0003_87f5).rotate_left(10) ^ b;
    b = b.wrapping_add(a).rotate_left(15) ^ 0x6a09_e667;
    a = a.wrapping_mul(0x0000_9e37).rotate_left(21) ^ b;
    b = b.wrapping_add(a).rotate_left(25) ^ 0xbb67_ae85;
    a = a.wrapping_mul(0x0000_79b9).rotate_left(27) ^ b;
    b = b.wrapping_add(a).rotate_left(30) ^ 0x3c6e_f372;
    a = a.wrapping_mul(0x0002_4b1d).rotate_left(1) ^ b;
    b = b.wrapping_add(a).rotate_left(4) ^ 0xa54f_f53a;
    a = a.wrapping_mul(0x0001_85ae).rotate_left(8) ^ b;
    b = b.wrapping_add(a).rotate_left(12) ^ 0x510e_527f;
    a = a.wrapping_mul(0x0003_c6ef).rotate_left(14) ^ b;
    b = b.wrapping_add(a).rotate_left(16) ^ 0x9b05_688c;
    a = a.wrapping_mul(0x0001_f83d).rotate_left(18) ^ b;
    b = b.wrapping_add(a).rotate_left(20) ^ 0x1f83_d9ab;
    a = a.wrapping_mul(0x0002_9b05).rotate_left(22) ^ b;
    b = b.wrapping_add(a).rotate_left(24) ^ 0x5be0_cd19;
    a = a.wrapping_mul(0x0003_5be0).rotate_left(26) ^ b;
    b = b.wrapping_add(a).rotate_left(28) ^ 0x428a_2f98;
    a ^ b
}

#[no_mangle]
pub fn rust_entry() {
    print_u32(churn(1));
    print_u32(churn(0xdead_beef));
}
