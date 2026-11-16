//! Differential test fixture covering 64-bit `(lo, hi)` pair arithmetic, shifts/rotates,
//! division/remainder, checked arithmetic, bit intrinsics, comparisons, `f32`/`f64` bit-exact
//! conversions (including `i64`/`u64`/`i128`/`u128 -> f32` without double-rounding and saturating
//! `f32`/`f64 -> i64`/`u64`), and `[u8; 8]` / struct-field round-trips.
//!
//! Compiled both as a `#![no_std]` JS `cdylib` (via `scripts/compile_core.sh`) and included by the
//! native `rustc -O --cfg oracle` binary in `scripts/native-i64-pair-differential.sh`.

#![cfg_attr(not(oracle), no_std)]
#![cfg_attr(not(oracle), no_main)]

#[cfg(not(oracle))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}

#[derive(Clone, Copy)]
struct PairState {
    a: u64,
    b: i64,
    tag: u32,
}

#[inline(never)]
fn inner_u64_to_f32_bits(x: u64) -> u32 {
    (x as f32).to_bits()
}

#[inline(never)]
fn inner_i64_to_f32_bits(x: i64) -> u32 {
    (x as f32).to_bits()
}

#[inline(never)]
fn inner_u64_to_f64_bits(x: u64) -> u64 {
    (x as f64).to_bits()
}

#[inline(never)]
fn inner_i64_to_f64_bits(x: i64) -> u64 {
    (x as f64).to_bits()
}

#[inline(never)]
fn inner_u128_to_f32_bits(x: u128) -> u32 {
    (x as f32).to_bits()
}

#[inline(never)]
fn inner_i128_to_f32_bits(x: i128) -> u32 {
    (x as f32).to_bits()
}

#[inline(never)]
fn inner_f64_bits_to_i64(bits: u64) -> i64 {
    f64::from_bits(bits) as i64
}

#[inline(never)]
fn inner_f64_bits_to_u64(bits: u64) -> u64 {
    f64::from_bits(bits) as u64
}

#[inline(never)]
fn inner_f32_bits_to_i64(bits: u32) -> i64 {
    f32::from_bits(bits) as i64
}

#[inline(never)]
fn inner_f32_bits_to_u64(bits: u32) -> u64 {
    f32::from_bits(bits) as u64
}

#[inline(never)]
fn inner_u64_add(a: u64, b: u64) -> u64 {
    a.wrapping_add(b)
}

#[inline(never)]
fn inner_u64_sub(a: u64, b: u64) -> u64 {
    a.wrapping_sub(b)
}

#[inline(never)]
fn inner_u64_mul(a: u64, b: u64) -> u64 {
    a.wrapping_mul(b)
}

#[inline(never)]
fn inner_u64_mul_consts(a: u64) -> u64 {
    a.wrapping_mul(3)
        .wrapping_add(a.wrapping_mul(5))
        .wrapping_add(a.wrapping_mul(9))
        .wrapping_add(a.wrapping_mul(16))
}

#[inline(never)]
fn inner_u64_div(a: u64, b: u64) -> u64 {
    if b == 0 { 0 } else { a.wrapping_div(b) }
}

#[inline(never)]
fn inner_u64_rem(a: u64, b: u64) -> u64 {
    if b == 0 { 0 } else { a.wrapping_rem(b) }
}

#[inline(never)]
fn inner_i64_div(a: i64, b: i64) -> i64 {
    if b == 0 { 0 } else { a.wrapping_div(b) }
}

#[inline(never)]
fn inner_i64_rem(a: i64, b: i64) -> i64 {
    if b == 0 { 0 } else { a.wrapping_rem(b) }
}

#[inline(never)]
fn inner_u64_shl(a: u64, s: u32) -> u64 {
    a.wrapping_shl(s)
}

#[inline(never)]
fn inner_u64_shr(a: u64, s: u32) -> u64 {
    a.wrapping_shr(s)
}

#[inline(never)]
fn inner_i64_shr(a: i64, s: u32) -> i64 {
    a.wrapping_shr(s)
}

#[inline(never)]
fn inner_u64_rol(a: u64, s: u32) -> u64 {
    a.rotate_left(s)
}

#[inline(never)]
fn inner_u64_ror(a: u64, s: u32) -> u64 {
    a.rotate_right(s)
}

#[inline(never)]
fn inner_u64_const_shifts(a: u64) -> u64 {
    (a << 33) ^ (a >> 33) ^ a.rotate_left(13) ^ a.rotate_right(37)
}

#[inline(never)]
fn inner_i64_const_shr(a: i64) -> i64 {
    (a >> 33) ^ (a >> 1)
}

#[inline(never)]
fn inner_checked_flags(a: u64, b: u64) -> u32 {
    let mut mask = 0u32;
    if a.checked_add(b).is_some() {
        mask |= 1;
    }
    if a.checked_sub(b).is_some() {
        mask |= 2;
    }
    if a.checked_mul(b).is_some() {
        mask |= 4;
    }
    let sa = a as i64;
    let sb = b as i64;
    if sa.checked_add(sb).is_some() {
        mask |= 8;
    }
    if sa.checked_sub(sb).is_some() {
        mask |= 16;
    }
    if sa.checked_mul(sb).is_some() {
        mask |= 32;
    }
    mask
}

#[inline(never)]
fn inner_checked_vals(a: u64, b: u64) -> u64 {
    let f = 0x9e37_79b9_7f4a_7c15u64;
    let sa = a as i64;
    let sb = b as i64;
    let u0 = a.checked_add(b).unwrap_or(f);
    let u1 = a.checked_sub(b).unwrap_or(f);
    let u2 = a.checked_mul(b).unwrap_or(f);
    let s0 = sa.checked_add(sb).unwrap_or(f as i64) as u64;
    let s1 = sa.checked_sub(sb).unwrap_or(f as i64) as u64;
    let s2 = sa.checked_mul(sb).unwrap_or(f as i64) as u64;
    u0.rotate_left(3)
        ^ u1.rotate_left(7)
        ^ u2.rotate_left(11)
        ^ s0.rotate_left(17)
        ^ s1.rotate_left(23)
        ^ s2.rotate_left(31)
}

#[inline(never)]
fn inner_u64_bit_counts(a: u64) -> u32 {
    a.count_ones() | (a.leading_zeros() << 8) | (a.trailing_zeros() << 16)
}

#[inline(never)]
fn inner_u64_swap_bytes(a: u64) -> u64 {
    a.swap_bytes()
}

#[inline(never)]
fn inner_u64_reverse_bits(a: u64) -> u64 {
    a.reverse_bits()
}

#[inline(never)]
fn inner_cmp_mask(a: u64, b: u64) -> u32 {
    let sa = a as i64;
    let sb = b as i64;
    let mut m = 0u32;
    if a == b {
        m |= 1 << 0;
    }
    if a != b {
        m |= 1 << 1;
    }
    if a < b {
        m |= 1 << 2;
    }
    if a <= b {
        m |= 1 << 3;
    }
    if a > b {
        m |= 1 << 4;
    }
    if a >= b {
        m |= 1 << 5;
    }
    if sa == sb {
        m |= 1 << 6;
    }
    if sa != sb {
        m |= 1 << 7;
    }
    if sa < sb {
        m |= 1 << 8;
    }
    if sa <= sb {
        m |= 1 << 9;
    }
    if sa > sb {
        m |= 1 << 10;
    }
    if sa >= sb {
        m |= 1 << 11;
    }
    m
}

#[inline(never)]
fn step_pair_state(mut s: PairState) -> PairState {
    let mut bytes = s.a.to_le_bytes();
    bytes.swap(0, 7);
    bytes[3] ^= (s.b & 0xff) as u8;
    let rebuilt = u64::from_le_bytes(bytes);
    let be = rebuilt.to_be_bytes();
    s.a = u64::from_be_bytes(be);
    s.b = s.b.wrapping_add(s.a as i64);
    s.tag = (s.tag.wrapping_add(7)) & 63;
    s
}

#[inline(never)]
fn inner_bytes_struct_roundtrip(a: u64, b: i64) -> u64 {
    let init = PairState {
        a,
        b,
        tag: (a as u32) & 31,
    };
    let next = step_pair_state(init);
    next.a ^ (next.b as u64).rotate_left(next.tag)
}

#[unsafe(no_mangle)]
pub fn u64_to_f32_bits(x: u64) -> u32 {
    inner_u64_to_f32_bits(x)
}

#[unsafe(no_mangle)]
pub fn i64_to_f32_bits(x: i64) -> u32 {
    inner_i64_to_f32_bits(x)
}

#[unsafe(no_mangle)]
pub fn u64_to_f64_bits(x: u64) -> u64 {
    inner_u64_to_f64_bits(x)
}

#[unsafe(no_mangle)]
pub fn i64_to_f64_bits(x: i64) -> u64 {
    inner_i64_to_f64_bits(x)
}

#[unsafe(no_mangle)]
pub fn u128_to_f32_bits(x: u128) -> u32 {
    inner_u128_to_f32_bits(x)
}

#[unsafe(no_mangle)]
pub fn i128_to_f32_bits(x: i128) -> u32 {
    inner_i128_to_f32_bits(x)
}

#[unsafe(no_mangle)]
pub fn f64_bits_to_i64(bits: u64) -> i64 {
    inner_f64_bits_to_i64(bits)
}

#[unsafe(no_mangle)]
pub fn f64_bits_to_u64(bits: u64) -> u64 {
    inner_f64_bits_to_u64(bits)
}

#[unsafe(no_mangle)]
pub fn f32_bits_to_i64(bits: u32) -> i64 {
    inner_f32_bits_to_i64(bits)
}

#[unsafe(no_mangle)]
pub fn f32_bits_to_u64(bits: u32) -> u64 {
    inner_f32_bits_to_u64(bits)
}

#[unsafe(no_mangle)]
pub fn u64_add(a: u64, b: u64) -> u64 {
    inner_u64_add(a, b)
}

#[unsafe(no_mangle)]
pub fn u64_sub(a: u64, b: u64) -> u64 {
    inner_u64_sub(a, b)
}

#[unsafe(no_mangle)]
pub fn u64_mul(a: u64, b: u64) -> u64 {
    inner_u64_mul(a, b)
}

#[unsafe(no_mangle)]
pub fn u64_mul_consts(a: u64) -> u64 {
    inner_u64_mul_consts(a)
}

#[unsafe(no_mangle)]
pub fn u64_div(a: u64, b: u64) -> u64 {
    inner_u64_div(a, b)
}

#[unsafe(no_mangle)]
pub fn u64_rem(a: u64, b: u64) -> u64 {
    inner_u64_rem(a, b)
}

#[unsafe(no_mangle)]
pub fn i64_div(a: i64, b: i64) -> i64 {
    inner_i64_div(a, b)
}

#[unsafe(no_mangle)]
pub fn i64_rem(a: i64, b: i64) -> i64 {
    inner_i64_rem(a, b)
}

#[unsafe(no_mangle)]
pub fn u64_shl(a: u64, s: u32) -> u64 {
    inner_u64_shl(a, s)
}

#[unsafe(no_mangle)]
pub fn u64_shr(a: u64, s: u32) -> u64 {
    inner_u64_shr(a, s)
}

#[unsafe(no_mangle)]
pub fn i64_shr(a: i64, s: u32) -> i64 {
    inner_i64_shr(a, s)
}

#[unsafe(no_mangle)]
pub fn u64_rol(a: u64, s: u32) -> u64 {
    inner_u64_rol(a, s)
}

#[unsafe(no_mangle)]
pub fn u64_ror(a: u64, s: u32) -> u64 {
    inner_u64_ror(a, s)
}

#[unsafe(no_mangle)]
pub fn u64_const_shifts(a: u64) -> u64 {
    inner_u64_const_shifts(a)
}

#[unsafe(no_mangle)]
pub fn i64_const_shr(a: i64) -> i64 {
    inner_i64_const_shr(a)
}

#[unsafe(no_mangle)]
pub fn checked_flags(a: u64, b: u64) -> u32 {
    inner_checked_flags(a, b)
}

#[unsafe(no_mangle)]
pub fn checked_vals(a: u64, b: u64) -> u64 {
    inner_checked_vals(a, b)
}

#[unsafe(no_mangle)]
pub fn u64_bit_counts(a: u64) -> u32 {
    inner_u64_bit_counts(a)
}

#[unsafe(no_mangle)]
pub fn u64_swap_bytes(a: u64) -> u64 {
    inner_u64_swap_bytes(a)
}

#[unsafe(no_mangle)]
pub fn u64_reverse_bits(a: u64) -> u64 {
    inner_u64_reverse_bits(a)
}

#[unsafe(no_mangle)]
pub fn cmp_mask(a: u64, b: u64) -> u32 {
    inner_cmp_mask(a, b)
}

#[unsafe(no_mangle)]
pub fn bytes_struct_roundtrip(a: u64, b: i64) -> u64 {
    inner_bytes_struct_roundtrip(a, b)
}

#[cfg(not(oracle))]
#[unsafe(no_mangle)]
pub fn rust_entry() {}
