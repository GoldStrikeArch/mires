//! Direct `AggregateKind::RawPtr` / `from_raw_parts` / `from_raw_parts_mut` from array pointers,
//! `MaybeUninit<[T; N]>`, nested `[[T; K]; M]`, byte arrays to `str`, sized pointees, and erased
//! `*const ()` pointers (MIRES-022).
//!
//! Directly targets MIR cast folding where rustc elides intermediate `PtrToPtr` casts feeding
//! `AggregateKind::RawPtr`, across both JIR (including `slice_forward`) and baseline/v1 lanes.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::rc::Rc;
use alloc::string::String;
use core::mem::MaybeUninit;
use core::ptr;
use core::slice;

#[inline(never)]
fn test_maybeuninit_string_array() -> u32 {
    let mut stack_array = MaybeUninit::<[String; 24]>::uninit();
    let scratch = unsafe {
        slice::from_raw_parts_mut(stack_array.as_mut_ptr() as *mut MaybeUninit<String>, 24)
    };
    for (i, slot) in scratch.iter_mut().enumerate() {
        let mut s = String::new();
        s.push(char::from(b'a' + (i as u8)));
        s.push(char::from(b'A' + ((23 - i) as u8)));
        slot.write(s);
    }
    // Copy element 23 to element 0 via copy_nonoverlapping (the exact smallsort pattern),
    // after moving out element 0 cleanly.
    let mut checksum = 0u32;
    unsafe {
        let first = scratch[0].assume_init_read();
        checksum = checksum.wrapping_add(first.as_bytes()[0] as u32);
        drop(first);

        let src = scratch.as_ptr().add(23) as *const String;
        let dst = scratch.as_mut_ptr() as *mut String;
        ptr::copy_nonoverlapping(src, dst, 1);

        // Read back slot 0 and slots 1..23 (skipping moved-from slot 23).
        for i in 0..23 {
            let s = scratch[i].assume_init_read();
            checksum = checksum.wrapping_add((s.as_bytes()[0] as u32) * ((i + 1) as u32));
        }
    }
    checksum
}

#[inline(never)]
fn test_maybeuninit_u32_and_slice_forward(idx: usize) -> u32 {
    let mut stack_array = MaybeUninit::<[u32; 16]>::uninit();
    unsafe {
        let dst = slice::from_raw_parts_mut(stack_array.as_mut_ptr() as *mut MaybeUninit<u32>, 16);
        for (i, slot) in dst.iter_mut().enumerate() {
            slot.write((i as u32 + 1) * 10);
        }
        // Immutable `SliceView` directly from `stack_array.as_ptr()` indexed by `idx`:
        // ensures `slice_forward` does NOT bypass `__rt.unwindow`!
        let ro = slice::from_raw_parts(stack_array.as_ptr() as *const u32, 16);
        let a = ro[idx];
        let b = ro[15 - idx];
        a + b
    }
}

#[inline(never)]
fn test_window_array_ptr_to_slice() -> u32 {
    let backing: [u32; 8] = [10, 20, 30, 40, 50, 60, 70, 80];
    unsafe {
        // Window slot `{ buf: backing, off: 2, w: 4 }` cast to `*const [u32; 4]`, then rebuilt
        // as a slice `&[u32]` of length 4.
        let win: *const [u32; 4] = backing.as_ptr().add(2) as *const [u32; 4];
        let s = slice::from_raw_parts(win as *const u32, 4);
        s[0] + s[1] + s[2] + s[3]
    }
}

#[inline(never)]
fn test_nested_array_chunk_slice() -> u32 {
    let mut arr2d: [[u32; 4]; 3] = [[1, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12]];
    unsafe {
        let p: *mut [[u32; 4]; 3] = &raw mut arr2d;
        let chunks: &mut [[u32; 4]] = slice::from_raw_parts_mut(p as *mut [u32; 4], 3);
        chunks[1][2] = 77;
        chunks[0][0] + chunks[1][2] + chunks[2][3]
    }
}

#[inline(never)]
fn test_array_ptr_to_str() -> u32 {
    let bytes: [u8; 8] = *b"mires022";
    unsafe {
        let p_arr: *const [u8; 8] = &raw const bytes;
        let s1 = core::str::from_utf8_unchecked(slice::from_raw_parts(p_arr as *const u8, 8));
        // Direct `*const str` built from `*const [u8; 8]` via `ptr::from_raw_parts` (unsized str).
        let raw_s: *const str = ptr::slice_from_raw_parts(p_arr as *const u8, 8) as *const str;
        let s2 = &*raw_s;
        let mut mu = MaybeUninit::<[u8; 5]>::uninit();
        let ms = slice::from_raw_parts_mut(mu.as_mut_ptr() as *mut u8, 5);
        ms.copy_from_slice(b"hello");
        let s3 = core::str::from_utf8_unchecked(slice::from_raw_parts(mu.as_ptr() as *const u8, 5));
        print_str(s1);
        print_str(s2);
        print_str(s3);
        (s1 == s2) as u32 + (s3 == "hello") as u32
    }
}

#[inline(never)]
fn test_erased_ptr_byte_add_slice() -> u32 {
    let arr: [u32; 6] = [100, 200, 300, 400, 500, 600];
    unsafe {
        // `byte_add` on `*const [u32]` erases through `*const u8` -> `*const ()`, then rebuilds
        // `*const [u32]` via `AggregateKind::RawPtr([u32], ...)` from `*const ()`.
        let base_slice: *const [u32] = &arr[..4];
        let shifted: *const [u32] = base_slice.byte_add(8);
        let s = &*shifted;
        s[0] + s[1] + s[2] + s[3]
    }
}

#[inline(never)]
fn test_rc_unsized_slice_alloc() -> u32 {
    // `Rc::<[T]>::from` allocates a fresh `*mut u8` heap block of `header + N * size_of::<T>()`
    // bytes (not divisible by 12 for `N = 2` `String`s: `8 + 24 = 32`), builds `*mut [T]` from it,
    // and reshapes via `__rt.retype_rc`. Must NOT prematurely retype the fresh block.
    let rc_s: Rc<[String]> = Rc::from([String::from("ab"), String::from("cde")]);
    let rc_u: Rc<[u32]> = Rc::from([7u32, 11, 13]);
    (rc_s[0].len() + rc_s[1].len()) as u32 + rc_u[0] + rc_u[1] + rc_u[2]
}

#[no_mangle]
pub extern "C" fn rust_entry() {
    print_u32(test_maybeuninit_string_array());
    print_u32(test_maybeuninit_u32_and_slice_forward(3));
    print_u32(test_maybeuninit_u32_and_slice_forward(11));
    print_u32(test_window_array_ptr_to_slice());
    print_u32(test_nested_array_chunk_slice());
    print_u32(test_array_ptr_to_str());
    print_u32(test_erased_ptr_byte_add_slice());
    print_u32(test_rc_unsized_slice_alloc());
}
