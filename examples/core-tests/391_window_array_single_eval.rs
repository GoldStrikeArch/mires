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

#[repr(transparent)]
#[derive(Copy, Clone)]
struct Byte(u8);

#[derive(Copy, Clone)]
struct Pair {
    a: i32,
    b: i32,
}

#[repr(transparent)]
#[derive(Copy, Clone)]
struct PairWrap(Pair);

#[inline(never)]
fn get_null_byte() -> *const Byte {
    black_box(ptr::null())
}

#[inline(never)]
fn get_dangling_byte() -> *const Byte {
    black_box(0x1000usize as *const Byte)
}

#[inline(never)]
fn test_reborrows_and_transparent_addr_of() {
    // Scaled byte pointer reborrow preserves `.sc` (regression check for JsPlace::Slot).
    let mut arr = [10u32, 20, 30, 40];
    unsafe {
        let r: &u8 = &*(addr_of!(arr[0]) as *const u8);
        let p_next: *const u32 = (r as *const u8).add(4) as *const u32;
        print_u32(*p_next);
        print_bool(ptr::eq(p_next, addr_of!(arr[1])));

        let p: *const u8 = addr_of!((*(addr_of!(arr[0]) as *const Byte)).0);
        let p_next2: *const u32 = p.add(4) as *const u32;
        print_u32(*p_next2);
        print_bool(ptr::eq(p_next2, addr_of!(arr[1])));

        // Reborrow through a window of scaled byte pointers (`&mut *(*win)[i]`).
        let mut ptrs: [*mut u8; 4] = [
            arr.as_mut_ptr() as *mut u8,
            arr.as_mut_ptr() as *mut u8,
            arr.as_mut_ptr() as *mut u8,
            arr.as_mut_ptr() as *mut u8,
        ];
        let win: *mut [*mut u8; 2] = addr_of_mut!(ptrs[1]) as *mut [*mut u8; 2];
        let i = black_box(1usize);
        let rw: &mut u8 = &mut *(*win)[i];
        let p_next3: *mut u32 = (rw as *mut u8).add(12) as *mut u32;
        print_u32(*p_next3);
        print_bool(ptr::eq(p_next3, addr_of!(arr[3])));
    }

    // Transparent-field `addr_of!` on null and dangling addresses (direct and through a window).
    unsafe {
        let pn: *const u8 = addr_of!((*get_null_byte()).0);
        print_bool(pn.is_null());
        let pd: *const u8 = addr_of!((*get_dangling_byte()).0);
        print_usize(pd as usize);

        let mut bptrs: [*const Byte; 4] = [
            ptr::null(),
            0x1000usize as *const Byte,
            ptr::null(),
            0x2000usize as *const Byte,
        ];
        let bwin: *mut [*const Byte; 2] = addr_of_mut!(bptrs[1]) as *mut [*const Byte; 2];
        let i0 = black_box(0usize);
        let i1 = black_box(1usize);
        let w_dang: *const u8 = addr_of!((*(*bwin)[i0]).0);
        print_usize(w_dang as usize);
        let w_null: *const u8 = addr_of!((*(*bwin)[i1]).0);
        print_bool(w_null.is_null());
    }

    // Transparent-field `addr_of_mut!` on an indirect wrapper stored behind a window of pointers.
    let mut pairs = [
        PairWrap(Pair { a: 1, b: 2 }),
        PairWrap(Pair { a: 3, b: 4 }),
        PairWrap(Pair { a: 5, b: 6 }),
        PairWrap(Pair { a: 7, b: 8 }),
    ];
    unsafe {
        let mut pptrs: [*mut PairWrap; 4] = [
            addr_of_mut!(pairs[0]),
            addr_of_mut!(pairs[1]),
            addr_of_mut!(pairs[2]),
            addr_of_mut!(pairs[3]),
        ];
        let pwin: *mut [*mut PairWrap; 2] = addr_of_mut!(pptrs[1]) as *mut [*mut PairWrap; 2];
        let i = black_box(0usize);
        let q: *mut Pair = addr_of_mut!((*(*pwin)[i]).0);
        print_bool(ptr::eq(q, addr_of!(pairs[1].0)));
        print_i32((*q).a);
        print_i32((*q).b);
        print_i32((*q.add(1)).a);
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    test_single_eval();
    test_erased_roundtrip_and_try_into();
    test_reborrows_and_transparent_addr_of();
}
