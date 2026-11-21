//! Chained projections, transparent wrappers, indirect struct elements, nested
//! arrays, `from_end` constant indices, and subslices through window-slot array
//! pointers.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::mem::MaybeUninit;
use core::ptr::{self, addr_of, addr_of_mut};

#[derive(Copy, Clone)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(transparent)]
struct Wrap([u8; 4]);

#[inline(never)]
fn test_transparent_wrappers() {
    // #[repr(transparent)] struct Wrap([u8; 4]) window at non-zero offset.
    let mut big = [1u8, 2, 3, 4, 5, 6, 7, 8];
    let wptr: *mut Wrap = addr_of_mut!(big[2]) as *mut Wrap;
    let i = black_box(2usize);
    unsafe {
        (*wptr).0[i] = 91;
        (*wptr).0[0] = 31;
        print_u32((*wptr).0[0] as u32);
        print_u32((*wptr).0[i] as u32);
        let r = &mut (*wptr).0[1];
        *r = 41;
        print_bool(ptr::eq(addr_of!((*wptr).0[i]), addr_of!(big[4])));
    }
    print_u32(big[2] as u32);
    print_u32(big[3] as u32);
    print_u32(big[4] as u32);

    // MaybeUninit<[u8; 4]> transparent union wrapper window at non-zero offset.
    let mut buf = [10u8, 20, 30, 40, 50, 60];
    let mu_arr: *mut MaybeUninit<[u8; 4]> = addr_of_mut!(buf[1]) as *mut MaybeUninit<[u8; 4]>;
    unsafe {
        let inner: &mut [u8; 4] = (*mu_arr).assume_init_mut();
        inner[i] = 88;
        print_u32(buf[3] as u32);
    }

    // [MaybeUninit<u8>; 4] window (the 25_array_ref shape) at non-zero offset.
    let mut mb = [MaybeUninit::<u8>::uninit(); 8];
    let mwin: *mut [MaybeUninit<u8>; 4] = addr_of_mut!(mb[2]) as *mut [MaybeUninit<u8>; 4];
    unsafe {
        (*mwin)[0] = MaybeUninit::new(b'A');
        (*mwin)[i] = MaybeUninit::new(b'Z');
        print_u32((*mwin)[0].assume_init() as u32);
        print_u32((*mwin)[i].assume_init() as u32);
        print_u32(mb[2].assume_init() as u32);
        print_u32(mb[4].assume_init() as u32);
    }
}

#[inline(never)]
fn test_struct_elements() {
    let mut pts = [
        Point { x: 1, y: 2 },
        Point { x: 3, y: 4 },
        Point { x: 5, y: 6 },
        Point { x: 7, y: 8 },
        Point { x: 9, y: 10 },
        Point { x: 11, y: 12 },
    ];
    let pwin: *mut [Point; 4] = addr_of_mut!(pts[2]) as *mut [Point; 4];
    let i = black_box(1usize);
    unsafe {
        // Direct field write & read through window element.
        (*pwin)[i].x = 42;
        (*pwin)[i].y += 10;
        print_i32((*pwin)[i].x);
        print_i32((*pwin)[i].y);
        print_i32(pts[3].x);
        print_i32(pts[3].y);

        // Mutable reference to indirect struct element aliases backing array.
        let r: &mut Point = &mut (*pwin)[i];
        r.x = 77;
        print_i32(pts[3].x);
        *r = Point { x: 111, y: 222 };
        print_i32(pts[3].x);
        print_i32(pts[3].y);

        // Raw pointer to struct element preserves identity.
        let rp: *mut Point = addr_of_mut!((*pwin)[2]);
        print_bool(ptr::eq(rp, addr_of!(pts[4])));
        (*rp).y = 333;
        print_i32(pts[4].y);
    }
}

#[inline(never)]
fn test_nested_arrays() {
    let mut mat = [[1u32, 2], [3, 4], [5, 6], [7, 8]];
    let mwin: *mut [[u32; 2]; 2] = addr_of_mut!(mat[1]) as *mut [[u32; 2]; 2];
    let i = black_box(1usize);
    let j = black_box(1usize);
    unsafe {
        (*mwin)[i][j] = 666;
        print_u32((*mwin)[i][j]);
        print_u32(mat[2][1]);
        let row: &mut [u32; 2] = &mut (*mwin)[0];
        row[0] = 333;
        print_u32(mat[1][0]);
        let cell: *mut u32 = addr_of_mut!((*mwin)[i][0]);
        print_bool(ptr::eq(cell, addr_of!(mat[2][0])));
        *cell = 555;
        print_u32(mat[2][0]);
    }
}

#[inline(never)]
fn test_from_end_and_subslice() {
    let mut big = [10u8, 20, 30, 40, 50, 60, 70, 80];
    let win: *mut [u8; 4] = addr_of_mut!(big[3]) as *mut [u8; 4];
    unsafe {
        // ConstantIndex with from_end: true on non-zero-offset window.
        let [first, .., prev, last] = *win;
        print_u32(first as u32);
        print_u32(prev as u32);
        print_u32(last as u32);

        // Subslice by value (from_end: true).
        let [a, mid @ .., d] = *win;
        print_u32(a as u32);
        print_u32(mid[0] as u32);
        print_u32(mid[1] as u32);
        print_u32(d as u32);

        // Subslice by mutable reference: mutating `mid` must update `big[4..6]`.
        let [_, ref mut mid_mut @ .., _] = *win;
        mid_mut[0] = 150;
        mid_mut[1] = 160;
        print_u32(big[4] as u32);
        print_u32(big[5] as u32);

        // Range slice borrows &(&(*win))[1..3] and &mut (&mut (*win))[1..3].
        let s: &[u8] = &(&(*win))[1..3];
        print_u32(s[0] as u32);
        print_u32(s[1] as u32);
        let sm: &mut [u8] = &mut (&mut (*win))[1..3];
        sm[0] = 201;
        sm[1] = 202;
        print_u32(big[4] as u32);
        print_u32(big[5] as u32);
    }

    // Zero-length array pointer sanity check.
    let mut empty: [u8; 0] = [];
    let ep: *mut [u8; 0] = addr_of_mut!(empty);
    unsafe {
        let r: &[u8; 0] = &*ep;
        print_u32(r.len() as u32);
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    test_transparent_wrappers();
    test_struct_elements();
    test_nested_arrays();
    test_from_end_and_subslice();
}
