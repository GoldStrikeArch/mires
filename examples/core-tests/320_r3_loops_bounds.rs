//! End-to-end fixture for Slice R3-B: scalarized slice/&str/RawParts iterators,
//! signed `<` step `+1` overflow folding, `chunks_exact`/subslice bounds elision,
//! and negative/edge controls.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[inline(never)]
fn slice_bytes_sum(s: &[u8]) -> usize {
    let mut sum = 0usize;
    for &b in s.iter() {
        sum += b as usize;
    }
    sum
}

#[inline(never)]
fn str_bytes_sum(s: &str) -> usize {
    let mut sum = 0usize;
    for b in s.as_bytes().iter() {
        sum += *b as usize;
    }
    sum
}

#[inline(never)]
fn raw_parts_sum(ptr: *const u8, len: usize) -> usize {
    let slice = unsafe { core::slice::from_raw_parts(ptr, len) };
    let mut sum = 0usize;
    for (i, &b) in slice.iter().enumerate() {
        if b == 0 {
            break;
        }
        sum += (i + 1) * (b as usize);
    }
    sum
}

#[inline(never)]
fn mut_slice_inc_and_sum(s: &mut [u8]) -> usize {
    for b in s.iter_mut() {
        *b = b.wrapping_add(1);
    }
    let mut sum = 0usize;
    for &b in s.iter() {
        sum += b as usize;
    }
    sum
}

#[inline(never)]
fn signed_i32_loop_sum(start: i32, end: i32) -> i32 {
    let mut i = start;
    let mut acc: i32 = 0;
    while i < end {
        acc = acc.wrapping_add(i);
        i += 1;
    }
    acc
}

#[inline(never)]
fn signed_i64_loop_sum(start: i64, end: i64) -> i64 {
    let mut i = start;
    let mut acc: i64 = 0;
    while i < end {
        acc = acc.wrapping_add(i);
        i += 1;
    }
    acc
}

#[inline(never)]
fn signed_i32_step2_guarded(start: i32, end: i32) -> i32 {
    let mut i = start;
    let mut count = 0;
    while i < end {
        count += 1;
        i = i.saturating_add(2);
    }
    count
}

#[inline(always)]
fn fixed4_from_parts(ptr: *const u8) -> usize {
    let s = unsafe { core::slice::from_raw_parts(ptr, 4) };
    (s[0] as usize) + (s[1] as usize) * 2 + (s[2] as usize) * 3 + (s[3] as usize) * 4
}

#[inline(never)]
fn fixed_raw_parts_bounds_elided(data: &[u8]) -> usize {
    if data.len() < 4 {
        return 0;
    }
    fixed4_from_parts(data.as_ptr())
}

#[inline(never)]
fn chunks_exact_checksum(data: &[u8], chunk_size: usize) -> usize {
    let iter = data.chunks_exact(chunk_size);
    let rem = iter.remainder();
    let mut total = 0usize;
    for chunk in iter {
        let mut h = 0usize;
        for &b in chunk {
            h = h.wrapping_mul(31).wrapping_add(b as usize);
        }
        total = total.wrapping_add(h);
    }
    for &b in rem {
        total = total.wrapping_add(b as usize * 7);
    }
    total
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let bytes = [3u8, 1, 4, 1, 5, 9, 2, 6, 5, 3, 5, 8, 9, 7, 9, 3, 2, 3, 8];
    print_usize(slice_bytes_sum(&bytes));
    print_usize(str_bytes_sum("hello r3-loops-bounds!"));
    print_usize(raw_parts_sum(bytes.as_ptr(), bytes.len()));

    let mut mut_buf = [10u8, 20, 30, 40];
    print_usize(mut_slice_inc_and_sum(&mut mut_buf));
    // Verify original array bytes are modified as expected.
    print_usize(mut_buf[0] as usize + mut_buf[3] as usize);

    // Signed `<` step `+1` positive + boundary edge cases (`i32::MIN`, `i32::MAX - 2..i32::MAX`).
    print_i32(signed_i32_loop_sum(0, 10));
    print_i32(signed_i32_loop_sum(i32::MIN, i32::MIN + 3));
    print_i32(signed_i32_loop_sum(i32::MAX - 2, i32::MAX));
    print_i64(signed_i64_loop_sum(-5, 6));
    print_i32(signed_i32_step2_guarded(i32::MAX - 4, i32::MAX - 1));

    // Range/RawParts fixed bounds + chunks_exact exact, non-multiple, and short slice.
    print_usize(fixed_raw_parts_bounds_elided(&bytes));
    print_usize(fixed_raw_parts_bounds_elided(&[1, 2]));
    print_usize(chunks_exact_checksum(&bytes[..16], 4));
    print_usize(chunks_exact_checksum(&bytes, 8));
    print_usize(chunks_exact_checksum(&bytes[..3], 8));
}
