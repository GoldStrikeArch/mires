//! Single-evaluation guarantees for window-slot array projections, erased-pointer
//! round-trips, and slice `try_into` conversions at non-zero offsets.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::ptr::{self, addr_of, addr_of_mut};

#[inline(never)]
fn next_win(buf: &mut [u32; 8], calls: &mut u32) -> *mut [u32; 4] {
    *calls += 1;
    addr_of_mut!(buf[2]) as *mut [u32; 4]
}

#[inline(never)]
fn next_idx(calls: &mut u32, v: usize) -> usize {
    *calls += 1;
    black_box(v)
}

#[inline(never)]
fn test_single_eval() {
    let mut big = [10u32, 20, 30, 40, 50, 60, 70, 80];
    let mut win_calls = 0u32;
    let mut idx_calls = 0u32;

    unsafe {
        // 1. Dynamic index write.
        let i = next_idx(&mut idx_calls, 2);
        (*next_win(&mut big, &mut win_calls))[i] = 123;
        print_u32(win_calls);
        print_u32(idx_calls);
        print_u32(big[4]);

        // 2. Dynamic index read.
        let v = (*next_win(&mut big, &mut win_calls))[next_idx(&mut idx_calls, 2)];
        print_u32(win_calls);
        print_u32(idx_calls);
        print_u32(v);

        // 3. Mutable element borrow.
        let r = &mut (*next_win(&mut big, &mut win_calls))[next_idx(&mut idx_calls, 1)];
        *r = 456;
        print_u32(win_calls);
        print_u32(idx_calls);
        print_u32(big[3]);

        // 4. Raw element pointer.
        let p = addr_of_mut!((*next_win(&mut big, &mut win_calls))[next_idx(&mut idx_calls, 3)]);
        print_u32(win_calls);
        print_u32(idx_calls);
        print_bool(ptr::eq(p, addr_of!(big[5])));
        *p = 789;
        print_u32(big[5]);

        // 5. Subslice mutable borrow.
        let [_, ref mut mid @ .., _] = *next_win(&mut big, &mut win_calls);
        mid[0] = 321;
        mid[1] = 654;
        print_u32(win_calls);
        print_u32(big[3]);
        print_u32(big[4]);

        // 6. from_end constant index.
        let [.., last] = *next_win(&mut big, &mut win_calls);
        print_u32(win_calls);
        print_u32(last);
    }
}

#[inline(never)]
fn test_erased_roundtrip_and_try_into() {
    let mut big = [100u32, 200, 300, 400, 500, 600, 700, 800];
    let win: *mut [u32; 4] = addr_of_mut!(big[2]) as *mut [u32; 4];
    let erased: *mut () = win as *mut ();
    let back: *mut [u32; 4] = erased as *mut [u32; 4];
    let i = black_box(2usize);
    unsafe {
        (*back)[i] = 999;
        print_u32((*back)[i]);
        print_u32(big[4]);
    }

    // Slice try_into at non-zero offset followed by direct writes & reads.
    let mut bytes = [1u8, 2, 3, 4, 5, 6, 7, 8];
    let slice: &mut [u8] = &mut bytes[2..6];
    let arr: &mut [u8; 4] = slice.try_into().unwrap();
    let j = black_box(1usize);
    arr[0] = 33;
    arr[j] = 44;
    arr[3] = 66;
    print_u32(bytes[2] as u32);
    print_u32(bytes[3] as u32);
    print_u32(bytes[5] as u32);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    test_single_eval();
    test_erased_roundtrip_and_try_into();
}
