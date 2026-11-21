//! Indexing, writing, reading, and borrowing elements through dereferenced slot
//! pointers to fixed-size arrays (`*mut [E; N]`, `*const [E; N]`, `Box<[E; N]>`).
//!
//! Covers both window slots (`{ buf, off, w }`, at zero and non-zero offsets, on
//! stack arrays and heap `Vec` buffers) and plain slots (`{ buf, off }`), plus
//! whole-place reads/writes/borrows alongside element projections.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;
use alloc::vec;
use core::hint::black_box;
use core::ptr::{self, addr_of, addr_of_mut};

#[inline(never)]
fn test_zero_offset_window() {
    let mut big = [10u8; 8];
    let win: *mut [u8; 4] = &mut big as *mut [u8; 8] as *mut [u8; 4];
    let i = black_box(2usize);
    unsafe {
        (*win)[i] = 99;
        (*win)[0] = 77;
        print_u32((*win)[i] as u32);
    }
    print_u32(big[0] as u32);
    print_u32(big[2] as u32);
}

#[inline(never)]
fn test_nonzero_offset_window_and_borrows() {
    let mut big = [10u8, 20, 30, 40, 50, 60, 70, 80];
    let base: *mut u8 = addr_of_mut!(big[3]);
    let win: *mut [u8; 4] = base as *mut [u8; 4];
    let i = black_box(2usize);

    unsafe {
        // Dynamic & constant index reads before write.
        print_u32((*win)[0] as u32);
        print_u32((*win)[i] as u32);
        print_u32((*win)[3] as u32);

        // Dynamic & constant index writes.
        (*win)[i] = 155;
        (*win)[0] = 144;
        print_u32(big[3] as u32);
        print_u32(big[5] as u32);

        // Shared and mutable references to elements.
        let r_ref: &u8 = &(*win)[i];
        print_u32(*r_ref as u32);
        let r_mut: &mut u8 = &mut (*win)[i];
        *r_mut = 166;
        print_u32(big[5] as u32);

        // Raw element pointers preserve backing buffer identity and offsets.
        let p_const: *const u8 = addr_of!((*win)[i]);
        let p_mut: *mut u8 = addr_of_mut!((*win)[1]);
        print_bool(ptr::eq(p_const, addr_of!(big[5])));
        print_bool((p_const as usize) == (addr_of!(big[5]) as usize));
        print_i32(p_const.offset_from(addr_of!(big[3])) as i32);
        *p_mut = 177;
        print_u32(big[4] as u32);
        print_u32(*p_mut.add(1) as u32);

        // Whole-place read, borrow, and write alongside indexed access.
        let whole: [u8; 4] = *win;
        print_u32(whole[0] as u32 + whole[1] as u32 + whole[2] as u32 + whole[3] as u32);
        let arr_ref: &mut [u8; 4] = &mut *win;
        arr_ref[3] = 188;
        print_u32(big[6] as u32);
        *win = [1, 2, 3, 4];
        print_u32(big[3] as u32 + big[4] as u32 + big[5] as u32 + big[6] as u32);
        // Neighbors outside the window are untouched.
        print_u32(big[2] as u32);
        print_u32(big[7] as u32);
    }
}

#[inline(never)]
fn test_plain_slot_array() {
    let mut arr = [5u8, 6, 7, 8];
    let plain: *mut [u8; 4] = addr_of_mut!(arr);
    let i = black_box(1usize);
    unsafe {
        (*plain)[i] = 66;
        (*plain)[3] = 88;
        print_u32((*plain)[i] as u32);
        let r = &mut (*plain)[0];
        *r = 55;
        let p = addr_of!((*plain)[i]);
        print_bool(ptr::eq(p, addr_of!(arr[1])));
        print_u32(arr[0] as u32 + arr[1] as u32 + arr[2] as u32 + arr[3] as u32);
    }
}

#[inline(never)]
fn test_box_and_vec_windows() {
    // Safe Rust: Box<[u32; 4]> is a heap window slot.
    let mut b: Box<[u32; 4]> = Box::new([100, 200, 300, 400]);
    let i = black_box(2usize);
    (*b)[i] = 333;
    (*b)[0] = 111;
    print_u32((*b)[0]);
    print_u32((*b)[i]);
    print_u32((*b)[3]);

    // Vec<u32> window at non-zero offset 3.
    let mut v = vec![10u32, 20, 30, 40, 50, 60, 70, 80];
    let vwin: *mut [u32; 4] = unsafe { v.as_mut_ptr().add(3) as *mut [u32; 4] };
    unsafe {
        (*vwin)[i] = 666;
        let r = &mut (*vwin)[1];
        *r = 555;
        let p = addr_of!((*vwin)[i]);
        print_bool(ptr::eq(p, addr_of!(v[5])));
        print_u32((*vwin)[0]);
        print_u32((*vwin)[1]);
        print_u32((*vwin)[2]);
        print_u32((*vwin)[3]);
    }
    print_u32(v[4]);
    print_u32(v[5]);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    test_zero_offset_window();
    test_nonzero_offset_window_and_borrows();
    test_plain_slot_array();
    test_box_and_vec_windows();
}
