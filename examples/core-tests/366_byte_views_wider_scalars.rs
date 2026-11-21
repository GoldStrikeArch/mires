//! Aliasing byte views (`u8` and `i8`) over wider primitive storage (MIRES-007).
//!
//! Covers `slice::from_raw_parts`, `slice::from_raw_parts_mut`, `align_to::<u8>()`,
//! `align_to_mut::<u8>()`, single-byte raw pointer reads/writes `*(p.cast::<u8>().add(k))`,
//! signed `i8` views, `compare_bytes` / slice equality against `&[u8]`, `copy_nonoverlapping`
//! into/from `[u8]`, `copy_within`, `write_bytes` / `fill`, live two-way aliasing, vector growth,
//! addresses, pointer equality, and round-trip casts back to the wider scalar type across
//! `u16`, `i16`, `u32`, `i32`, `u64`, `i64`, `u128`, `i128`, `f32`, `f64`, `char`, `usize`,
//! and `isize` on heap (`vec![]`, `Vec::new()` + `push`, `Box<[T]>`), stack arrays, non-zero
//! offsets, single locals, and struct fields.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
use core::ptr;
use core::slice;

fn fold_bytes(bytes: &[u8]) -> u64 {
    let mut acc = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        acc ^= u64::from(b);
        acc = acc.wrapping_mul(0x0000_0100_0000_01b3);
    }
    acc
}

fn check_bytes<T>(slice: &[T], expected: &[u8]) -> bool {
    let byte_len = core::mem::size_of_val(slice);
    if byte_len != expected.len() {
        return false;
    }
    let raw = unsafe { slice::from_raw_parts(slice.as_ptr().cast::<u8>(), byte_len) };
    if raw != expected {
        return false;
    }
    let (head, mid, tail) = unsafe { slice.align_to::<u8>() };
    if !head.is_empty() || !tail.is_empty() || mid != expected {
        return false;
    }
    for (k, &want) in expected.iter().enumerate() {
        let got = unsafe { *slice.as_ptr().cast::<u8>().add(k) };
        if got != want {
            return false;
        }
    }
    true
}

struct Holder {
    tag: u8,
    value: u32,
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    // 1. Read every wider primitive type through slice::from_raw_parts, align_to, and *p.add(k).
    let v_u16: Vec<u16> = black_box(vec![0x0102, 0x0304, 0xfeff]);
    let mut v_u16_push: Vec<u16> = Vec::new();
    v_u16_push.push(black_box(0x0102));
    v_u16_push.push(black_box(0x0304));
    v_u16_push.push(black_box(0xfeff));
    let a_u16: [u16; 3] = black_box([0x0102, 0x0304, 0xfeff]);
    let exp_u16 = [0x02, 0x01, 0x04, 0x03, 0xff, 0xfe];
    print_bool(check_bytes(&v_u16, &exp_u16));
    print_bool(check_bytes(&v_u16_push, &exp_u16));
    print_bool(check_bytes(&a_u16, &exp_u16));
    // Non-zero subslice offset.
    print_bool(check_bytes(&v_u16[1..], &exp_u16[2..]));
    print_bool(check_bytes(&v_u16_push[1..], &exp_u16[2..]));
    print_bool(check_bytes(&a_u16[1..], &exp_u16[2..]));

    // Direct slice::from_raw_parts on Vec::new()+push (GVN path under MIR inline + -Oz).
    let raw_push = unsafe { slice::from_raw_parts(v_u16_push.as_ptr().cast::<u8>(), 6) };
    print_bool(raw_push == &exp_u16);
    print_u64(fold_bytes(raw_push));

    // i16, u32, i32, u64, i64, u128, i128, f32, f64, char, usize, isize.
    let v_i16: Vec<i16> = black_box(vec![-1, i16::MIN, 0x0102]);
    let exp_i16 = [0xff, 0xff, 0x00, 0x80, 0x02, 0x01];
    print_bool(check_bytes(&v_i16, &exp_i16));

    let v_u32: Vec<u32> = black_box(vec![0x1234_5678, 0x8000_0001]);
    let exp_u32 = [0x78, 0x56, 0x34, 0x12, 0x01, 0x00, 0x00, 0x80];
    print_bool(check_bytes(&v_u32, &exp_u32));

    let v_i32: [i32; 2] = black_box([-1, i32::MIN]);
    let exp_i32 = [0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x80];
    print_bool(check_bytes(&v_i32, &exp_i32));

    let v_u64: Vec<u64> = black_box(vec![0x0102_0304_0506_0708, u64::MAX]);
    let exp_u64 = [
        0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff,
    ];
    print_bool(check_bytes(&v_u64, &exp_u64));

    let v_i64: Vec<i64> = black_box(vec![-1, i64::MIN]);
    let exp_i64 = [
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x80,
    ];
    print_bool(check_bytes(&v_i64, &exp_i64));

    let v_u128: Vec<u128> = black_box(vec![0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10]);
    let exp_u128 = [
        0x10, 0x0f, 0x0e, 0x0d, 0x0c, 0x0b, 0x0a, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02,
        0x01,
    ];
    print_bool(check_bytes(&v_u128, &exp_u128));

    let v_i128: [i128; 1] = black_box([i128::MIN]);
    let exp_i128 = [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x80,
    ];
    print_bool(check_bytes(&v_i128, &exp_i128));

    let v_f32: Vec<f32> = black_box(vec![1.0, -0.0, -2.5]);
    let exp_f32: [u8; 12] = [
        0x00, 0x00, 0x80, 0x3f, // 1.0f32
        0x00, 0x00, 0x00, 0x80, // -0.0f32
        0x00, 0x00, 0x20, 0xc0, // -2.5f32
    ];
    print_bool(check_bytes(&v_f32, &exp_f32));

    let v_f64: [f64; 3] = black_box([1.0, -0.0, -1.5]);
    let exp_f64: [u8; 24] = [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, 0x3f, // 1.0f64
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, // -0.0f64
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf8, 0xbf, // -1.5f64
    ];
    print_bool(check_bytes(&v_f64, &exp_f64));

    let v_char: Vec<char> = black_box(vec!['A', '€']);
    let exp_char = [0x41, 0x00, 0x00, 0x00, 0xac, 0x20, 0x00, 0x00];
    print_bool(check_bytes(&v_char, &exp_char));

    let v_usize: Vec<usize> = black_box(vec![0x1122_3344]);
    print_bool(check_bytes(&v_usize, &v_usize[0].to_le_bytes()));
    let v_isize: Vec<isize> = black_box(vec![-2]);
    print_bool(check_bytes(&v_isize, &v_isize[0].to_le_bytes()));

    // Single local and struct field.
    let single: u32 = black_box(0xdead_beef);
    let single_bytes = unsafe { slice::from_raw_parts((&single as *const u32).cast::<u8>(), 4) };
    print_bool(single_bytes == &[0xef, 0xbe, 0xad, 0xde]);

    let holder = black_box(Holder {
        tag: 7,
        value: 0x1122_3344,
    });
    let field_bytes =
        unsafe { slice::from_raw_parts((&holder.value as *const u32).cast::<u8>(), 4) };
    print_bool(field_bytes == &[0x44, 0x33, 0x22, 0x11]);
    print_i32(i32::from(holder.tag));

    // Signed i8 byte view over u16 storage.
    let signed_view = unsafe { slice::from_raw_parts(v_u16.as_ptr().cast::<i8>(), 6) };
    print_i32(i32::from(signed_view[0]));
    print_i32(i32::from(signed_view[4]));
    print_i32(i32::from(signed_view[5]));

    // 2. Live two-way aliasing & byte mutations across integer & float widths.
    let mut mut16: Vec<u16> = black_box(vec![0x0102, 0x0304]);
    let p16 = mut16.as_mut_ptr().cast::<u8>();
    unsafe {
        // Mutation through typed Vec is observed through existing byte pointer.
        mut16[0] = 0xaabb;
        print_i32(i32::from(*p16));
        print_i32(i32::from(*p16.add(1)));
        // Mutation through byte pointer is observed in typed Vec.
        *p16.add(1) = 0xcc;
        *p16.add(2) = 0xdd;
    }
    print_u32(u32::from(mut16[0]));
    print_u32(u32::from(mut16[1]));

    // align_to_mut + copy_from_slice + copy_within + fill.
    let mut mut_i16: Vec<i16> = black_box(vec![0, 0]);
    {
        let (_, mid, _) = unsafe { mut_i16.align_to_mut::<u8>() };
        mid.copy_from_slice(&[0xff, 0xff, 0x00, 0x80]);
    }
    print_i32(i32::from(mut_i16[0]));
    print_i32(i32::from(mut_i16[1]));

    {
        let bytes = unsafe { slice::from_raw_parts_mut(mut_i16.as_mut_ptr().cast::<u8>(), 4) };
        bytes.copy_within(2..4, 0);
    }
    print_i32(i32::from(mut_i16[0]));
    print_i32(i32::from(mut_i16[1]));

    let mut mut_i32: Vec<i32> = black_box(vec![0]);
    unsafe {
        let b = slice::from_raw_parts_mut(mut_i32.as_mut_ptr().cast::<u8>(), 4);
        b.fill(0xff);
    }
    print_i32(mut_i32[0]);

    let mut mut_i64: Vec<i64> = black_box(vec![0]);
    unsafe {
        let b = slice::from_raw_parts_mut(mut_i64.as_mut_ptr().cast::<u8>(), 8);
        b.copy_from_slice(&[0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    }
    print_i64(mut_i64[0]);

    let mut mut_i128: [i128; 1] = black_box([0]);
    unsafe {
        let b = slice::from_raw_parts_mut(mut_i128.as_mut_ptr().cast::<u8>(), 16);
        b[15] = 0x80;
    }
    print_bool(mut_i128[0] == i128::MIN);

    let mut mut_f32: Vec<f32> = black_box(vec![0.0]);
    unsafe {
        let b = slice::from_raw_parts_mut(mut_f32.as_mut_ptr().cast::<u8>(), 4);
        b.copy_from_slice(&[0x00, 0x00, 0xc0, 0x3f]); // 1.5f32
    }
    print_f64(f64::from(mut_f32[0]));

    let mut mut_f64: Vec<f64> = black_box(vec![1.5]);
    unsafe {
        let b = slice::from_raw_parts_mut(mut_f64.as_mut_ptr().cast::<u8>(), 8);
        b[7] |= 0x80; // flip sign bit
    }
    print_f64(mut_f64[0]);

    // 3. copy_nonoverlapping into [u8] and from [u8].
    let mut dst_u8 = [0u8; 6];
    unsafe {
        ptr::copy_nonoverlapping(v_u16.as_ptr().cast::<u8>(), dst_u8.as_mut_ptr(), 6);
    }
    print_bool(dst_u8 == exp_u16);

    let src_u8 = [0x11u8, 0x22, 0x33, 0x44];
    let mut dst_u16 = [0u16; 2];
    unsafe {
        ptr::copy_nonoverlapping(src_u8.as_ptr(), dst_u16.as_mut_ptr().cast::<u8>(), 4);
    }
    print_u32(u32::from(dst_u16[0]));
    print_u32(u32::from(dst_u16[1]));

    // 4. Growth (realloc), addresses, pointer equality, round-trip casts, str/is_ascii interop.
    let mut grow: Vec<u16> = black_box(vec![0x6261, 0x6463]); // "abcd"
    let before = unsafe { slice::from_raw_parts(grow.as_ptr().cast::<u8>(), 4) };
    print_bool(before.is_ascii());
    print_str(core::str::from_utf8(before).unwrap());

    grow.push(0x6665); // "ef"
    grow.push(0x6867); // "gh"
    let after = unsafe { slice::from_raw_parts(grow.as_ptr().cast::<u8>(), 8) };
    print_str(core::str::from_utf8(after).unwrap());

    unsafe {
        let p = grow.as_ptr();
        let bp = p.cast::<u8>();
        print_bool(bp.addr() == p.addr());
        print_bool(bp.add(2).addr() == p.add(1).addr());
        print_bool(bp.add(1).addr() == p.addr() + 1);
        print_bool(ptr::eq(bp, p.cast::<u8>()));
        print_bool(ptr::eq(bp.add(2).cast::<u16>(), p.add(1)));
        print_bool(ptr::eq(p.byte_add(4), p.add(2)));
        print_u32(u32::from(*bp.add(2).cast::<u16>()));
    }
}
