//! Byte arithmetic on retyped non-scalar allocations + nested prefix & byte-view polish (MIRES-008).
//!
//! Covers:
//! - Aligned whole-element byte-offset casts, `copy_nonoverlapping`, overlapping `copy`,
//!   `ptr::eq`, `addr`, `byte_offset_from`, and `realloc` on retyped `Pair`, nested
//!   `OuterPair`, `String`, `Vec<&Pair>`, and `#[repr(packed)] Packed4` allocations.
//! - Multi-level nested zero-offset prefix `_unwrap_view` & `ptr_cmp`.
//! - Prefix vs direct field `_byte_view` cache isolation (`scale_scalar` and `byte_sign`).
//! - `ptr::eq` on shared `_non_scalar_byte_view` (`sc === undefined` vs `sc === 8`).

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::alloc::{alloc, alloc_zeroed, dealloc, realloc, Layout};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
use core::ptr;
use core::slice;

#[repr(C)]
#[derive(Clone, Copy)]
struct Pair {
    a: u32,
    b: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OuterPair {
    p: Pair,
    tag: u32,
}

#[repr(packed)]
#[derive(Clone, Copy)]
struct Packed4 {
    a: u16,
    b: u16,
}

#[repr(C)]
struct Inner {
    x: u32,
    y: u32,
}

#[repr(C)]
struct Outer {
    inner: Inner,
    z: u64,
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    // 1. alloc_zeroed + alloc Pair: byte-offset reads/writes, ptr::eq, addr, byte_offset_from, copy, realloc.
    unsafe {
        let layout = Layout::array::<Pair>(4).unwrap();
        let a = alloc_zeroed(layout);
        let p = a as *mut Pair;
        *p.add(0) = black_box(Pair { a: 10, b: 11 });
        *p.add(1) = black_box(Pair { a: 20, b: 21 });
        *p.add(2) = black_box(Pair { a: 30, b: 31 });
        *p.add(3) = black_box(Pair { a: 40, b: 41 });

        let p0 = *(a.add(0) as *const Pair);
        let p1 = *(a.add(8) as *const Pair);
        print_u32(p0.a);
        print_u32(p0.b);
        print_u32(p1.a);
        print_u32(p1.b);

        *(a.add(16) as *mut Pair) = Pair { a: 99, b: 100 };
        print_u32((*p.add(2)).a);
        print_u32((*p.add(2)).b);

        print_bool((a.add(8) as usize) == (p.add(1) as usize));
        print_bool(ptr::eq(a.add(8), p.cast::<u8>().add(8)));
        print_bool(ptr::eq(a.add(8).cast::<()>(), p.add(1).cast::<()>()));
        print_bool(p.add(1).cast::<()>().addr() == a.add(8).addr());
        print_i32(a.add(8).byte_offset_from(p.cast::<u8>()) as i32);
        print_i32(p.add(1).cast::<u8>().byte_offset_from(a) as i32);

        // Copy into uninitialized alloc + mutate destination: source must remain independent.
        let b = alloc(layout);
        let pb = b as *mut Pair;
        ptr::copy_nonoverlapping(a.add(8), b.add(16), 8);
        (*pb.add(2)).a = 999;
        print_u32((*p.add(1)).a);
        print_u32((*pb.add(2)).a);
        print_u32((*pb.add(2)).b);
        print_bool(ptr::eq(b.add(16).cast::<()>(), pb.add(2).cast::<()>()));

        // Overlapping copy forward & backward within Pair allocation + mutation independence.
        ptr::copy(a.add(8), a.add(16), 16);
        (*p.add(2)).a = 888;
        print_u32((*p.add(1)).a);
        print_u32((*p.add(2)).a);
        print_u32((*p.add(3)).a);

        ptr::copy(a.add(16), a.add(8), 16);
        (*p.add(1)).a = 777;
        print_u32((*p.add(1)).a);
        print_u32((*p.add(2)).a);

        // Cross-pointer copy between raw alloc pointer and scaled element pointer.
        ptr::copy_nonoverlapping(a.add(0), pb.cast::<u8>().add(8), 8);
        print_u32((*pb.add(1)).a);
        print_u32((*pb.add(1)).b);

        // realloc grow & shrink on Pair allocation.
        let a_grown = realloc(a, layout, 64);
        let p_grown = a_grown as *mut Pair;
        ptr::write(p_grown.add(7), Pair { a: 70, b: 71 });
        print_u32((*(a_grown.add(56) as *const Pair)).a);
        let a_shrunk = realloc(a_grown, Layout::array::<Pair>(8).unwrap(), 16);
        print_u32((*(a_shrunk.add(8) as *const Pair)).a);
        dealloc(a_shrunk, Layout::array::<Pair>(2).unwrap());
        dealloc(b, layout);

        // Nested struct deep clone independence on byte copy.
        let lo = Layout::array::<OuterPair>(2).unwrap();
        let ao = alloc(lo);
        let po = ao as *mut OuterPair;
        ptr::write(
            po.add(0),
            OuterPair {
                p: Pair { a: 5, b: 6 },
                tag: 1,
            },
        );
        let bo = alloc(lo);
        let pbo = bo as *mut OuterPair;
        let sz = core::mem::size_of::<OuterPair>();
        ptr::copy_nonoverlapping(ao, bo.add(sz), sz);
        (*pbo.add(1)).p.a = 555;
        print_u32((*po.add(0)).p.a);
        print_u32((*pbo.add(1)).p.a);
        dealloc(ao, lo);
        dealloc(bo, lo);

        // Vec<&Pair> copy via *mut u8 preserves &Pair identity without overwriting borrowed targets.
        let local_pair = black_box(Pair { a: 42, b: 43 });
        let dummy_pair = black_box(Pair { a: 0, b: 0 });
        let mut vref: Vec<&Pair> = Vec::with_capacity(2);
        vref.push(&local_pair);
        vref.push(&local_pair);
        let mut dst_ref: Vec<&Pair> = Vec::with_capacity(2);
        dst_ref.push(&dummy_pair);
        dst_ref.push(&dummy_pair);
        ptr::copy_nonoverlapping(
            vref.as_ptr().cast::<u8>(),
            dst_ref.as_mut_ptr().cast::<u8>(),
            core::mem::size_of::<&Pair>() * 2,
        );
        print_bool(ptr::eq(
            dst_ref[0] as *const Pair,
            &local_pair as *const Pair,
        ));
        print_bool(ptr::eq(
            dst_ref[1] as *const Pair,
            &local_pair as *const Pair,
        ));
        print_u32(dummy_pair.a);

        // String allocation (non-Copy owned aggregate).
        let ls = Layout::array::<String>(2).unwrap();
        let as_ptr = alloc(ls);
        let ps = as_ptr as *mut String;
        ptr::write(ps.add(0), String::from("hello"));
        ptr::write(ps.add(1), String::from("world"));
        let s1: &String = &*(as_ptr.add(core::mem::size_of::<String>()) as *const String);
        print_str(s1.as_str());
        ptr::drop_in_place(ps.add(0));
        ptr::drop_in_place(ps.add(1));
        dealloc(as_ptr, ls);

        // Packed4 (align == 1) retyped allocation.
        let lp = Layout::array::<Packed4>(4).unwrap();
        let ap = alloc_zeroed(lp);
        let pp = ap as *mut Packed4;
        *pp.add(0) = Packed4 { a: 1, b: 2 };
        *pp.add(1) = Packed4 { a: 3, b: 4 };
        let v = *(ap.add(4) as *const Packed4);
        let va = v.a;
        let vb = v.b;
        print_u32(u32::from(va));
        print_u32(u32::from(vb));
        dealloc(ap, lp);
    }

    // 2. A7 polish #1: 2-level nested prefix Outer(16) -> Inner(8) -> u32 -> u8.
    let arr = black_box([
        Outer {
            inner: Inner { x: 1, y: 2 },
            z: 3,
        },
        Outer {
            inner: Inner {
                x: 0xfe22_3344,
                y: 0x5566_7788,
            },
            z: 9,
        },
    ]);
    unsafe {
        let p0 = arr.as_ptr();
        let p1 = arr.as_ptr().add(1);
        let px0 = black_box(black_box(p0.cast::<Inner>()).cast::<u32>());
        let px1 = black_box(black_box(p1.cast::<Inner>()).cast::<u32>());
        let pb0 = px0.cast::<u8>();
        let pb1 = px1.cast::<u8>();
        print_bool((pb1 as usize) == (p1 as usize));
        print_bool((pb1 as usize) == (px1 as usize));
        print_bool(pb1.addr() == px1.addr());
        print_bool(ptr::eq(pb1.cast::<()>(), px1.cast::<()>()));
        print_bool(pb1.add(2).addr() == px1.addr() + 2);
        print_bool(pb0 < pb1);
        print_bool(px0 < px1);
        print_bool(slice::from_raw_parts(pb1, 4) == &[0x44, 0x33, 0x22, 0xfe]);
        print_bool(pb1.cast::<i8>().addr() == (p1 as usize));
        print_i32(i32::from(*pb1.cast::<i8>().add(3)));
    }

    // 3. A7 polish #2: prefix vs direct field byte-view cache isolation (both orders + byte_sign).
    let w = black_box(Pair {
        a: 0xfead_beef,
        b: 0x1234_5678,
    });
    unsafe {
        let pf = (&w.a as *const u32).cast::<u8>();
        let pf_i8 = pf.cast::<i8>();
        let addr_before = pf.addr();
        let addr_i8_before = pf_i8.addr();
        let pp = black_box((&w as *const Pair).cast::<u32>()).cast::<u8>();
        let pp_i8 = pp.cast::<i8>();
        print_bool(pf.addr() == addr_before);
        print_bool(pf_i8.addr() == addr_i8_before);
        print_bool(pp.addr() == (&w as *const Pair as usize));
        print_bool(pp_i8.addr() == (&w as *const Pair as usize));
        print_bool(*pf == 0xef && *pp == 0xef);
        print_i32(i32::from(*pp_i8.add(3)));
    }
    let w2 = black_box(Pair {
        a: 0x0102_0304,
        b: 0x0506_0708,
    });
    unsafe {
        let pp = black_box((&w2 as *const Pair).cast::<u32>()).cast::<u8>();
        let pref_addr = pp.addr();
        let pf = (&w2.a as *const u32).cast::<u8>();
        print_bool(pf.addr() == (&w2.a as *const u32).addr());
        print_bool(pp.addr() == pref_addr);
    }

    // 4. A7 polish #3: ptr::eq on shared _non_scalar_byte_view (sc === undefined vs sc === 8).
    let v: Vec<Pair> = black_box(vec![Pair { a: 1, b: 2 }, Pair { a: 3, b: 4 }]);
    let p_scaled = v.as_ptr().cast::<u8>();
    let p_from_slice = unsafe { slice::from_raw_parts(v.as_ptr().cast::<u8>(), 0) }.as_ptr();
    print_bool(ptr::eq(p_scaled, p_from_slice));
    let p_scaled_1 = unsafe { v.as_ptr().add(1) }.cast::<u8>();
    let p_slice_1 = unsafe { slice::from_raw_parts(v.as_ptr().add(1).cast::<u8>(), 0) }.as_ptr();
    print_bool(ptr::eq(p_scaled_1, p_slice_1));
}
