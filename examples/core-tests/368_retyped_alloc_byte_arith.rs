//! Byte arithmetic on retyped scalar heap allocations (MIRES-008).
//!
//! Pins byte-offset re-casts (`a.add(k) as *const T`), unaligned scalar reads,
//! single-byte reads/writes, signed `i8` views, `slice::from_raw_parts` and
//! `compare_bytes`, `copy_nonoverlapping` (into/from `[u8]` and between retyped
//! allocations), overlapping forward/backward `copy`, aligned/unaligned
//! `write_bytes`, `ptr::eq`, `addr`, `byte_offset_from`, and `realloc` (grow
//! and shrink before and after retype) across `u16`, `i16`, `u32`, `i32`, `u64`,
//! `i64`, `u128`, `i128`, `f32`, `f64`, `char`, and `align == 1` raw allocations.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::alloc::{alloc, alloc_zeroed, dealloc, realloc, Layout};
use core::hint::black_box;
use core::ptr;
use core::slice;

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    // 1. u16 alloc + retype + full byte-arithmetic suite.
    unsafe {
        let layout = Layout::array::<u16>(4).unwrap();
        let a: *mut u8 = alloc(layout);
        let p = a as *mut u16;
        *p.add(0) = black_box(11);
        *p.add(1) = black_box(22);
        *p.add(2) = black_box(33);
        *p.add(3) = black_box(44);

        print_u32(u32::from(*(a.add(0) as *const u16)));
        print_u32(u32::from(*(a.add(2) as *const u16)));
        print_u32(u32::from(*(a.add(4) as *const u16)));
        print_u32(u32::from(*(a.add(6) as *const u16)));

        // Unaligned u16 read at byte offset 1: [0x00, 0x16] -> 0x1600 = 5632.
        print_u32(u32::from(ptr::read_unaligned(a.add(1) as *const u16)));

        // Single-byte reads & writes + signed i8 cast.
        print_u32(u32::from(*a.add(0)));
        print_u32(u32::from(*a.add(1)));
        print_u32(u32::from(*a.add(2)));
        print_u32(u32::from(*a.add(3)));

        // slice::from_raw_parts + compare_bytes.
        print_bool(slice::from_raw_parts(a.add(2), 4) == &[22, 0, 33, 0]);
        print_bool(slice::from_raw_parts(a.add(2), 2) == &[22, 0]);

        // copy_nonoverlapping: retyped -> [u8], [u8] -> retyped, retyped -> retyped.
        let mut dst_u8 = [0u8; 4];
        ptr::copy_nonoverlapping(a.add(2), dst_u8.as_mut_ptr(), 4);
        print_bool(dst_u8 == [22, 0, 33, 0]);

        let src_u8 = [0x01u8, 0x02, 0x03, 0x04];
        ptr::copy_nonoverlapping(src_u8.as_ptr(), a.add(2), 4);
        print_u32(u32::from(*p.add(1)));
        print_u32(u32::from(*p.add(2)));

        let a2 = alloc_zeroed(layout);
        let p2 = a2 as *mut u16;
        ptr::copy_nonoverlapping(a.add(2), a2.add(4), 4);
        print_u32(u32::from(*p2.add(0)));
        print_u32(u32::from(*p2.add(1)));
        print_u32(u32::from(*p2.add(2)));
        print_u32(u32::from(*p2.add(3)));

        // Overlapping copy forward & backward.
        *p.add(0) = 0x1111;
        *p.add(1) = 0x2222;
        *p.add(2) = 0x3333;
        *p.add(3) = 0x4444;
        ptr::copy(a.add(2), a.add(4), 4);
        print_u32(u32::from(*p.add(2)));
        print_u32(u32::from(*p.add(3)));
        ptr::copy(a.add(4), a.add(2), 4);
        print_u32(u32::from(*p.add(1)));
        print_u32(u32::from(*p.add(2)));

        // write_bytes aligned (zero & non-zero) and unaligned.
        ptr::write_bytes(a.add(2), 0, 4);
        print_u32(u32::from(*p.add(0)));
        print_u32(u32::from(*p.add(1)));
        print_u32(u32::from(*p.add(2)));
        print_u32(u32::from(*p.add(3)));

        ptr::write_bytes(a.add(2), 0xab, 4);
        print_u32(u32::from(*p.add(1)));
        print_u32(u32::from(*p.add(2)));

        *p.add(0) = 0;
        *p.add(1) = 0;
        ptr::write_bytes(a.add(1), 0xff, 2);
        print_u32(u32::from(*p.add(0)));
        print_u32(u32::from(*p.add(1)));

        // Signed i8 byte view over retyped u16 allocation.
        *p.add(0) = black_box(0xfe02);
        let ai = a.cast::<i8>();
        print_i32(i32::from(*ai.add(0)));
        print_i32(i32::from(*ai.add(1)));

        // ptr::eq, addr, byte_offset_from.
        print_bool(ptr::eq(a.add(2).cast::<()>(), p.add(1).cast::<()>()));
        print_bool((a.add(2) as usize) == (p.add(1) as usize));
        print_i32(a.add(2).byte_offset_from(p.cast::<u8>()) as i32);
        print_i32(p.add(1).cast::<u8>().byte_offset_from(a) as i32);

        // realloc after retype (grow & shrink).
        *p.add(0) = 100;
        *p.add(1) = 200;
        let a_grown = realloc(a, layout, 16);
        let p_grown = a_grown as *mut u16;
        *p_grown.add(7) = 700;
        print_u32(u32::from(*(a_grown.add(2) as *const u16)));
        print_u32(u32::from(*(a_grown.add(14) as *const u16)));
        let a_shrunk = realloc(a_grown, Layout::array::<u16>(8).unwrap(), 4);
        print_u32(u32::from(*(a_shrunk.add(2) as *const u16)));
        dealloc(a_shrunk, Layout::array::<u16>(2).unwrap());
        dealloc(a2, layout);

        // realloc before retype.
        let a_pre = alloc(Layout::array::<u16>(2).unwrap());
        let a_pre2 = realloc(a_pre, Layout::array::<u16>(2).unwrap(), 8);
        let p_pre = a_pre2 as *mut u16;
        *p_pre.add(0) = 77;
        *p_pre.add(1) = 88;
        print_u32(u32::from(*(a_pre2.add(2) as *const u16)));
        dealloc(a_pre2, layout);
    }

    // 2. i16, u32, i32, u64, i64, u128, i128, f32, f64, char.
    unsafe {
        let li16 = Layout::array::<i16>(2).unwrap();
        let ai16 = alloc(li16);
        let pi16 = ai16 as *mut i16;
        *pi16.add(0) = -1;
        *pi16.add(1) = -1234;
        print_i32(i32::from(*(ai16.add(2) as *const i16)));
        dealloc(ai16, li16);

        let l32 = Layout::array::<u32>(2).unwrap();
        let a32 = alloc(l32);
        let p32 = a32 as *mut u32;
        *p32.add(0) = 0x1122_3344;
        *p32.add(1) = 0x5566_7788;
        print_u32(*(a32.add(4) as *const u32));
        dealloc(a32, l32);

        let li32 = Layout::array::<i32>(2).unwrap();
        let ai32 = alloc(li32);
        let pi32 = ai32 as *mut i32;
        *pi32.add(0) = 1;
        *pi32.add(1) = -987_654;
        print_i32(*(ai32.add(4) as *const i32));
        dealloc(ai32, li32);

        let l64 = Layout::array::<u64>(2).unwrap();
        let a64 = alloc(l64);
        let p64 = a64 as *mut u64;
        *p64.add(0) = 10;
        *p64.add(1) = 0x0102_0304_0506_0708;
        print_u64(*(a64.add(8) as *const u64));
        dealloc(a64, l64);

        let li64 = Layout::array::<i64>(2).unwrap();
        let ai64 = alloc(li64);
        let pi64 = ai64 as *mut i64;
        *pi64.add(0) = 0;
        *pi64.add(1) = -42_000_000_000;
        print_i64(*(ai64.add(8) as *const i64));
        dealloc(ai64, li64);

        let lu128 = Layout::array::<u128>(2).unwrap();
        let au128 = alloc(lu128);
        let pu128 = au128 as *mut u128;
        *pu128.add(0) = 1;
        *pu128.add(1) = 0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10;
        print_bool(*(au128.add(16) as *const u128) == 0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10);
        dealloc(au128, lu128);

        let li128 = Layout::array::<i128>(2).unwrap();
        let ai128 = alloc(li128);
        let pi128 = ai128 as *mut i128;
        *pi128.add(0) = -1;
        *pi128.add(1) = -1_234_567_890_123_456_789;
        print_bool(*(ai128.add(16) as *const i128) == -1_234_567_890_123_456_789);
        dealloc(ai128, li128);

        let lf32 = Layout::array::<f32>(2).unwrap();
        let af32 = alloc(lf32);
        let pf32 = af32 as *mut f32;
        *pf32.add(0) = 0.5;
        *pf32.add(1) = -2.25;
        print_f64(f64::from(*(af32.add(4) as *const f32)));
        dealloc(af32, lf32);

        let lf64 = Layout::array::<f64>(2).unwrap();
        let af64 = alloc(lf64);
        let pf64 = af64 as *mut f64;
        *pf64.add(0) = 1.25;
        *pf64.add(1) = -3.5;
        print_f64(*(af64.add(8) as *const f64));
        dealloc(af64, lf64);

        let lch = Layout::array::<char>(2).unwrap();
        let ach = alloc(lch);
        let pch = ach as *mut char;
        *pch.add(0) = 'A';
        *pch.add(1) = '€';
        print_bool(*(ach.add(4) as *const char) == '€');
        dealloc(ach, lch);
    }

    // 3. align == 1 raw allocation retyped to u16.
    unsafe {
        let l1 = Layout::from_size_align(8, 1).unwrap();
        let a = alloc(l1);
        let p = a as *mut u16;
        *p.add(0) = black_box(11);
        *p.add(1) = black_box(22);
        *p.add(2) = black_box(33);
        *p.add(3) = black_box(44);
        print_u32(u32::from(*(a.add(2) as *const u16)));
        print_u32(u32::from(ptr::read_unaligned(a.add(1) as *const u16)));
        print_bool(slice::from_raw_parts(a.add(2), 4) == &[22, 0, 33, 0]);
        print_bool((a.add(2) as usize) == (p.add(1) as usize));
        print_bool(ptr::eq(a.add(2).cast::<()>(), p.add(1).cast::<()>()));
        ptr::write_bytes(a.add(2), 0xab, 4);
        print_u32(u32::from(*p.add(1)));
        print_u32(u32::from(*p.add(2)));
        dealloc(a, l1);
    }
}
