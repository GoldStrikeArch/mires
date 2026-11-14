//! Checks 64-bit byte-array transmutations (`from_le_bytes`, `from_be_bytes`,
//! `to_le_bytes`, `to_be_bytes`), `f64`/`f32` bitcasts and numeric casts to/from
//! `i64`/`u64`, `i128`/`u128` boundary conversions, and `switch` over `i64`/`u64`.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[inline(never)]
fn classify_u64(v: u64) -> i32 {
    match v {
        0 => 10,
        1 => 11,
        0x8000_0000_0000_0000 => 12,
        0xffff_ffff_ffff_ffff => 13,
        0x0123_4567_89ab_cdef => 14,
        _ => -1,
    }
}

#[inline(never)]
fn float_cast_roundtrip(f: f64) -> i64 {
    let x = f as i64;
    let u = f as u64;
    x.wrapping_add(u as i64)
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. Byte conversions (LE / BE)
    let raw: [u8; 8] = [0xef, 0xcd, 0xab, 0x89, 0x67, 0x45, 0x23, 0x01];
    let le = u64::from_le_bytes(raw);
    let be = u64::from_be_bytes(raw);
    print_u64(le);
    print_u64(be);
    let back_le = le.to_le_bytes();
    let back_be = be.to_be_bytes();
    print_u32(
        (back_le[0] as u32)
            | ((back_le[3] as u32) << 8)
            | ((back_be[0] as u32) << 16)
            | ((back_be[7] as u32) << 24),
    );
    let sle = i64::from_le_bytes([0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f]);
    print_i64(sle);

    // 2. Float bitcasts & numeric casts (including saturating edges & precision > 2^53)
    let pi = core::f64::consts::PI;
    let bits = pi.to_bits();
    print_u64(bits);
    print_u64(f64::from_bits(bits).to_bits());
    print_u64(f64::from_bits(0x8000_0000_0000_0000u64).to_bits());

    let big_i = 9_007_199_254_740_995i64; // 2^53 + 3, tests IEEE-754 ties-to-even!
    let big_u = 18_446_744_073_709_551_615u64;
    print_u64((big_i as f64).to_bits());
    print_u64((9_007_199_254_740_994i64 as f64).to_bits()); // 2^53 + 2 (exact!)
    print_u64((9_007_199_254_740_993i64 as f64).to_bits()); // 2^53 + 1 -> ties to even!
    print_u64((big_u as f64).to_bits());
    print_u64(((-big_i) as f64).to_bits());
    print_u32(((0x1234_5678_9abc_def0i64) as f32).to_bits());
    print_u32(((-0x1234_5678_9abc_def1i64) as f32).to_bits());
    print_u32(((u64::MAX) as f32).to_bits());

    print_i64(float_cast_roundtrip(1.0e20));
    print_i64(float_cast_roundtrip(-1.0e20));
    print_i64(float_cast_roundtrip(f64::NAN));
    print_i64(float_cast_roundtrip(-12345.678));
    print_u64(1.0e20_f32 as u64);
    print_i64((-1.0e20_f32) as i64);

    // 3. Int <-> Int casts (bool, i8, u16, i32, u32, i64, u64, i128, u128)
    let neg1: i8 = -5;
    let u16v: u16 = 0xf00d;
    print_i64(neg1 as i64);
    print_u64(neg1 as u64);
    print_i64(u16v as i64);
    print_i32(0x1234_5678_9abc_def0u64 as i32);
    print_u32(0x1234_5678_9abc_def0u64 as u32);
    let w128: i128 = -0x1_0000_0000_0000_0005i128;
    print_i64(w128 as i64);
    print_u64((w128 as u128) as u64);
    let back128 = (sle as i128).wrapping_mul(2);
    print_i64((back128 >> 1) as i64);

    // 4. SwitchInt on u64 / i64
    print_i32(classify_u64(0));
    print_i32(classify_u64(1));
    print_i32(classify_u64(0x8000_0000_0000_0000));
    print_i32(classify_u64(u64::MAX));
    print_i32(classify_u64(0x0123_4567_89ab_cdef));
    print_i32(classify_u64(99));
}
