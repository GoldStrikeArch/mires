//! `core::num`: the wrapping / checked / saturating / overflowing families, the bit methods, and
//! the 64 bit types, which are `BigInt` on the JavaScript side.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Constants, at every width this backend represents differently.
    print_i32(i32::MAX);
    print_i32(i32::MIN);
    print_u32(u32::MAX);
    print_i64(i64::MAX);
    print_i64(i64::MIN);
    print_u64(u64::MAX);
    print_usize(usize::MAX);
    print_usize(usize::BITS as usize);

    // Wrapping arithmetic, which is where the masking has to be exactly right.
    print_i32(i32::MAX.wrapping_add(1));
    print_i32(i32::MIN.wrapping_sub(1));
    print_i32(i32::MAX.wrapping_mul(3));
    print_u32(0u32.wrapping_sub(1));
    print_u32(u32::MAX.wrapping_add(2));
    print_i32((-7i32).wrapping_div(2));
    print_i32((-7i32).wrapping_rem(2));
    print_i64(i64::MAX.wrapping_add(1));
    print_u64(0u64.wrapping_sub(1));

    // Checked arithmetic, which is `*_with_overflow` underneath.
    print_i32(i32::MAX.checked_add(1).unwrap_or(-1));
    print_i32(3i32.checked_add(4).unwrap());
    print_i32(1i32.checked_div(0).unwrap_or(-1));
    print_bool(i32::MIN.checked_neg().is_none());
    print_i64(i64::MAX.checked_mul(2).unwrap_or(-1));
    print_u32(5u32.checked_sub(9).unwrap_or(0));

    // Overflowing returns the pair.
    let (value, overflowed) = i32::MAX.overflowing_add(1);
    print_i32(value);
    print_bool(overflowed);
    let (value, overflowed) = 2i32.overflowing_mul(3);
    print_i32(value);
    print_bool(overflowed);

    // Saturating clamps instead.
    print_i32(i32::MAX.saturating_add(1));
    print_i32(i32::MIN.saturating_sub(1));
    print_u32(0u32.saturating_sub(5));
    print_i64(i64::MAX.saturating_mul(2));

    // Bit methods.
    print_u32(0b1011u32.count_ones());
    print_u32(0b1011u32.count_zeros());
    print_u32(1u32.leading_zeros());
    print_u32(8u32.trailing_zeros());
    print_u32(0x12345678u32.swap_bytes());
    print_u32(0x1u32.rotate_left(4));
    print_u32(0x80000000u32.rotate_left(1));
    print_i32((-1i64 as u64).count_ones() as i32);
    print_u64(0x1234567890abcdefu64.swap_bytes());

    // Sign, absolute value, exponentiation.
    print_i32((-9i32).abs());
    print_i32((-9i32).signum());
    print_i32(0i32.signum());
    print_i32(2i32.pow(10));
    print_i64(3i64.pow(20));
    print_u32(7u32.next_power_of_two());
    print_bool(16u32.is_power_of_two());

    // Euclidean division, which differs from the truncating one on negatives.
    print_i32((-7i32).div_euclid(2));
    print_i32((-7i32).rem_euclid(2));
    print_i32((-7i32) / 2);
    print_i32((-7i32) % 2);

    // Casts across the number / BigInt boundary.
    print_i64(i32::MIN as i64);
    print_i32(i64::MAX as i32);
    print_u64(u32::MAX as u64);
    print_i32(-1i64 as i32);
    print_u32(-1i32 as u32);
    print_i32(300i32 as u8 as i32);
    print_i32(-1i32 as u8 as i32);
    print_u64(1u64 << 40);
    print_u64((1u64 << 63) >> 60);
    print_i64((-1i64) >> 60);

    // Floats, including the saturating float-to-integer cast.
    print_f64(2.5f64 + 0.25);
    print_f64(7.0f64 / 2.0);
    print_i32(1e30f64 as i32);
    print_i32(-1e30f64 as i32);
    print_i32(f64::NAN as i32);
    print_i32(3.99f64 as i32);
    print_i32(-3.99f64 as i32);
    print_f64((0.5f32 + 0.25f32) as f64);

    // `NonZero`, whose `new` is a `transmute` straight into the niche encoded
    // `Option<NonZero<T>>` — the decode the backend has to perform at run time.
    let nz = core::num::NonZeroU32::new(5).unwrap();
    print_u32(nz.get());
    print_u32(nz.leading_zeros());
    print_bool(core::num::NonZeroU32::new(0).is_none());
    print_u64(core::num::NonZeroU64::new(1 << 40).unwrap().get());
}
