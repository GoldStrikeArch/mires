//! Lazy proxy-free `Vec` + raw `alloc` byte arithmetic (`align == 1` and `align > 1`).
//!
//! Pins both ordinary `Vec<u8>`/`Vec<u32>`/`Vec<Pair>` growth/shrink and raw `alloc`/`alloc_zeroed`
//! byte-pointer arithmetic without eager `Proxy` wrapping: pre- and post-retype `addr`/`ptr::eq`,
//! inline `*a.add(k)` byte reads/writes, `&*a.add(k)` single-byte borrows, raw `*const [u8]` /
//! `*mut [i8]` slice indexing, `copy`/`copy_nonoverlapping` across scalar and non-scalar buffers,
//! `realloc` grow/shrink, and `dealloc` through either raw or typed pointers.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::alloc::{alloc, alloc_zeroed, dealloc, realloc, Layout};
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
use core::ptr;

#[repr(C)]
#[derive(Clone, Copy)]
struct Pair {
    a: u32,
    b: u32,
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    // 1. Ordinary Vec<u8>, Vec<u32>, Vec<Pair> growth, shrink, clone, pop.
    let mut vb: Vec<u8> = Vec::new();
    for i in 0u8..20 {
        vb.push(black_box(i));
    }
    vb.truncate(10);
    vb.shrink_to_fit();
    print_u32(vb.len() as u32);
    print_u32(u32::from(vb[9]));

    let mut vu: Vec<u32> = Vec::with_capacity(2);
    for i in 1u32..=12 {
        vu.push(black_box(i * 10));
    }
    vu.shrink_to_fit();
    print_u32(vu.len() as u32);
    print_u32(vu[0] + vu[11]);

    let mut vp: Vec<Pair> = vec![Pair { a: 1, b: 2 }, Pair { a: 3, b: 4 }];
    for i in 5u32..10 {
        vp.push(black_box(Pair { a: i, b: i + 100 }));
    }
    let vp_clone = vp.clone();
    vp[0].a = 999;
    print_u32(vp[0].a);
    print_u32(vp_clone[0].a);
    print_u32(vp_clone.last().unwrap().b);

    // 2. align > 1 raw alloc_zeroed + retype u32 + pre/post addr, single-byte borrow, raw slice
    //    indexing, copy_nonoverlapping, realloc, dealloc through typed pointer.
    unsafe {
        let layout = Layout::array::<u32>(4).unwrap();
        let a = alloc_zeroed(layout);
        let pre_addr = a.addr();
        let p = a as *mut u32;
        *p.add(0) = black_box(0x1122_3344);
        *p.add(1) = black_box(0xfe80_0102);

        print_bool(a.addr() == pre_addr);
        print_bool(p.cast::<u8>().addr() == pre_addr);
        print_bool(ptr::eq(a.add(4), p.add(1).cast::<u8>()));

        // Inline *a.add(k) and single-byte reference &*a.add(k).
        let b0_ref: &u8 = &*a.add(0);
        let b3_ref: &u8 = &*a.add(3);
        print_u32(u32::from(*b0_ref));
        print_u32(u32::from(*b3_ref));

        // Raw *mut [u8] and *const [i8] slice indexing without going through &[u8].
        let raw_u8: *mut [u8] = ptr::slice_from_raw_parts_mut(a, 8);
        (*raw_u8)[1] = 0xaa;
        print_u32(*p.add(0));

        let raw_i8: *const [i8] = ptr::slice_from_raw_parts(a.cast::<i8>(), 8);
        print_i32(i32::from((*raw_i8)[7]));

        // copy_nonoverlapping + realloc grow/shrink + dealloc through p.cast::<u8>().
        let mut out = [0u8; 4];
        ptr::copy_nonoverlapping(a.add(4), out.as_mut_ptr(), 4);
        print_bool(out == [0x02, 0x01, 0x80, 0xfe]);

        let a_grown = realloc(a, layout, 32);
        let p_grown = a_grown as *mut u32;
        *p_grown.add(7) = 0xdead_beef;
        print_u32(*(a_grown.add(28) as *const u32));
        print_u32(u32::from(*a_grown.add(28)));

        let a_shrunk = realloc(a_grown, Layout::array::<u32>(8).unwrap(), 8);
        let p_shrunk = a_shrunk as *mut u32;
        print_u32(*p_shrunk.add(1));
        dealloc(p_shrunk.cast::<u8>(), Layout::array::<u32>(2).unwrap());
    }

    // 3. align == 1 raw alloc + retype u32 + byte reads/writes + copy + dealloc through raw ptr.
    unsafe {
        let l1 = Layout::from_size_align(8, 1).unwrap();
        let a = alloc(l1);
        let p = a as *mut u32;
        *p.add(0) = black_box(0x0102_0304);
        *p.add(1) = black_box(0x0506_0708);
        print_u32(u32::from(*a.add(0)));
        print_u32(u32::from(*a.add(4)));
        *a.add(4) = 0x99;
        print_u32(*p.add(1));

        let b = alloc_zeroed(l1);
        let pb = b as *mut u32;
        ptr::copy_nonoverlapping(a, b, 8);
        print_u32(*pb.add(0));
        print_u32(*pb.add(1));
        dealloc(a, l1);
        dealloc(b, l1);
    }

    // 4. Non-scalar Pair raw alloc + aligned whole-element byte copy preserves independence + realloc.
    unsafe {
        let lp = Layout::array::<Pair>(3).unwrap();
        let a = alloc(lp);
        let p = a as *mut Pair;
        *p.add(0) = black_box(Pair { a: 10, b: 20 });
        *p.add(1) = black_box(Pair { a: 30, b: 40 });

        let b = alloc(lp);
        let pb = b as *mut Pair;
        ptr::copy_nonoverlapping(a, b, 16);
        (*pb.add(0)).a = 777;
        print_u32((*p.add(0)).a);
        print_u32((*pb.add(0)).a);
        print_u32((*pb.add(1)).b);

        let a2 = realloc(a, lp, 40);
        let p2 = a2 as *mut Pair;
        *p2.add(4) = Pair { a: 50, b: 60 };
        print_u32((*(a2.add(32) as *const Pair)).b);
        dealloc(a2, Layout::array::<Pair>(5).unwrap());
        dealloc(b, lp);
    }
}
