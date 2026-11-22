//! Byte arithmetic on retyped non-scalar allocations + nested prefix & byte-view polish (MIRES-008).
//!
//! Covers:
//! - Aligned whole-element byte-offset casts, `copy`/`copy_nonoverlapping` (deep-cloning owned
//!   aggregates including nested structs, `Holder { r: &Pair }`, and `BufNamed` while rebinding
//!   `&Pair`/`&u32` slots directly without mutating referents or poisoning `_boxes`), `ptr::eq`,
//!   `addr`, `byte_offset_from`, and `realloc` on retyped `Pair`, `vec![Pair]`, `String`,
//!   and `#[repr(packed)] Packed4` allocations.
//! - Multi-level nested zero-offset prefix `_unwrap_view` & `ptr_cmp` (2-level stack and
//!   3-level heap).
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

#[repr(C)]
#[derive(Clone, Copy)]
struct Holder<'a> {
    r: &'a Pair,
    tag: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BufNamed {
    buf: u32,
    len: u32,
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

#[repr(C)]
struct TopOuter {
    outer: Outer,
    top: u64,
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    // 1. alloc_zeroed + alloc Pair: byte-offset reads/writes, ptr::eq, addr, byte_offset_from, realloc.
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

        // Aligned whole-element `*mut u8` copy/copy_nonoverlapping on Pair preserves independence.
        let b = alloc(layout);
        let pb = b as *mut Pair;
        ptr::copy_nonoverlapping(a.add(8), b.add(16), 8);
        (*pb.add(2)).a = 999;
        print_u32((*p.add(1)).a);
        print_u32((*pb.add(2)).a);
        print_u32((*pb.add(2)).b);
        print_bool(ptr::eq(b.add(16).cast::<()>(), pb.add(2).cast::<()>()));
        ptr::copy(a.add(8), a.add(16), 16);
        (*p.add(2)).a = 888;
        print_u32((*p.add(1)).a);
        print_u32((*p.add(2)).a);
        print_u32((*p.add(3)).a);

        // realloc grow & shrink on Pair allocation.
        let a_grown = realloc(a, layout, 64);
        let p_grown = a_grown as *mut Pair;
        ptr::write(p_grown.add(7), Pair { a: 70, b: 71 });
        print_u32((*(a_grown.add(56) as *const Pair)).a);
        print_bool(ptr::eq(
            a_grown.add(56).cast::<()>(),
            p_grown.add(7).cast::<()>(),
        ));
        let a_shrunk = realloc(a_grown, Layout::array::<Pair>(8).unwrap(), 16);
        print_u32((*(a_shrunk.add(8) as *const Pair)).a);
        dealloc(a_shrunk, Layout::array::<Pair>(2).unwrap());
        dealloc(b, layout);

        // vec![Pair { ... }] registers owned elements in _boxes (write_array / convert_nv_block),
        // while vec![&local] does NOT poison _boxes with borrowed referents.
        let vp_vec: Vec<Pair> = black_box(vec![Pair { a: 1, b: 2 }, Pair { a: 3, b: 4 }]);
        let vp = vp_vec.as_ptr();
        let vb = vp.cast::<u8>();
        print_bool(ptr::eq(vb.add(8).cast::<()>(), vp.add(1).cast::<()>()));
        print_bool(vb.add(8).addr() == vp.add(1).cast::<()>().addr());

        let local_p = black_box(Pair { a: 42, b: 43 });
        let dummy_p = black_box(Pair { a: 7, b: 8 });
        let vref: Vec<&Pair> = black_box(vec![&local_p, &local_p]);
        let mut dst_ref: Vec<&Pair> = Vec::with_capacity(2);
        ptr::copy_nonoverlapping(
            vref.as_ptr().cast::<u8>(),
            dst_ref.as_mut_ptr().cast::<u8>(),
            core::mem::size_of::<&Pair>() * 2,
        );
        dst_ref.set_len(2);
        print_bool(ptr::eq(dst_ref[0] as *const Pair, &local_p as *const Pair));
        print_bool(!ptr::eq(
            (&local_p as *const Pair).cast::<()>(),
            dst_ref.as_ptr().cast::<()>(),
        ));
        let mut arr_ref: [&Pair; 2] = black_box([&dummy_p, &dummy_p]);
        ptr::copy_nonoverlapping(
            vref.as_ptr().cast::<u8>(),
            arr_ref.as_mut_ptr().cast::<u8>(),
            core::mem::size_of::<&Pair>() * 2,
        );
        print_bool(ptr::eq(arr_ref[0] as *const Pair, &local_p as *const Pair));
        print_u32(dummy_p.a);

        // Vec<OuterPair>, Holder { r: &Pair }, and BufNamed byte copies preserve exact clone semantics.
        let vo1: Vec<OuterPair> = black_box(vec![
            OuterPair {
                p: Pair { a: 11, b: 12 },
                tag: 1,
            },
            OuterPair {
                p: Pair { a: 21, b: 22 },
                tag: 2,
            },
        ]);
        let mut vo2: Vec<OuterPair> = Vec::with_capacity(2);
        ptr::copy_nonoverlapping(
            vo1.as_ptr().cast::<u8>(),
            vo2.as_mut_ptr().cast::<u8>(),
            core::mem::size_of::<OuterPair>() * 2,
        );
        vo2.set_len(2);
        vo2[0].p.a = 999;
        print_u32(vo1[0].p.a);
        print_u32(vo2[0].p.a);

        let lh = Layout::array::<Holder>(1).unwrap();
        let ah = alloc(lh);
        let ph = ah as *mut Holder;
        ptr::write(
            ph,
            Holder {
                r: &local_p,
                tag: 5,
            },
        );
        let bh = alloc(lh);
        let pbh = bh as *mut Holder;
        ptr::copy_nonoverlapping(ah, bh, core::mem::size_of::<Holder>());
        (*pbh).tag = 99;
        print_bool(ptr::eq((*pbh).r as *const Pair, &local_p as *const Pair));
        print_u32((*ph).tag);
        print_u32((*pbh).tag);
        dealloc(ah, lh);
        dealloc(bh, lh);

        let lbn = Layout::array::<BufNamed>(2).unwrap();
        let abn = alloc(lbn);
        let pbn = abn as *mut BufNamed;
        ptr::write(pbn.add(0), BufNamed { buf: 10, len: 20 });
        ptr::copy(
            abn.add(0),
            abn.add(core::mem::size_of::<BufNamed>()),
            core::mem::size_of::<BufNamed>(),
        );
        (*pbn.add(1)).buf = 777;
        print_u32((*pbn.add(0)).buf);
        print_u32((*pbn.add(1)).buf);
        dealloc(abn, lbn);

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
        let vb_p = v.b;
        print_u32(u32::from(va));
        print_u32(u32::from(vb_p));
        print_bool((ap.add(4) as usize) == (pp.add(1) as usize));
        print_bool(ptr::eq(ap.add(4).cast::<()>(), pp.add(1).cast::<()>()));
        dealloc(ap, lp);

        // 3-level nested prefix on heap allocation TopOuter -> Outer -> Inner -> u32 -> u8.
        let lt = Layout::array::<TopOuter>(2).unwrap();
        let at = alloc(lt);
        let pt = at as *mut TopOuter;
        ptr::write(
            pt.add(0),
            TopOuter {
                outer: Outer {
                    inner: Inner { x: 1, y: 2 },
                    z: 3,
                },
                top: 4,
            },
        );
        ptr::write(
            pt.add(1),
            TopOuter {
                outer: Outer {
                    inner: Inner {
                        x: 0x1122_3344,
                        y: 5,
                    },
                    z: 6,
                },
                top: 7,
            },
        );
        let pt1 = pt.add(1);
        let px3 =
            black_box(black_box(black_box(pt1.cast::<Outer>()).cast::<Inner>()).cast::<u32>());
        let pb3 = px3.cast::<u8>();
        print_bool(ptr::eq(pb3.cast::<()>(), pt1.cast::<()>()));
        print_bool(ptr::eq(
            pb3.cast::<()>(),
            at.add(core::mem::size_of::<TopOuter>()).cast::<()>(),
        ));
        print_bool(pb3.addr() == pt1.cast::<()>().addr());
        dealloc(at, lt);
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
    let pp2 = black_box((&w2 as *const Pair).cast::<u32>()).cast::<u8>();
    let pref_addr = pp2.addr();
    let pf2 = (&w2.a as *const u32).cast::<u8>();
    print_bool(pf2.addr() == (&w2.a as *const u32).addr());
    print_bool(pp2.addr() == pref_addr);

    // 4. A7 polish #3: ptr::eq on shared _non_scalar_byte_view (sc === undefined vs sc === 8)
    //    and erased roundtrip ptr::eq(p.add(1), p.add(1).cast::<()>().cast::<T>()).
    let v: Vec<Pair> = black_box(vec![Pair { a: 1, b: 2 }, Pair { a: 3, b: 4 }]);
    let p_scaled = v.as_ptr().cast::<u8>();
    let p_from_slice = unsafe { slice::from_raw_parts(v.as_ptr().cast::<u8>(), 0) }.as_ptr();
    print_bool(ptr::eq(p_scaled, p_from_slice));
    let p_scaled_1 = unsafe { v.as_ptr().add(1) }.cast::<u8>();
    let p_slice_1 = unsafe { slice::from_raw_parts(v.as_ptr().add(1).cast::<u8>(), 0) }.as_ptr();
    print_bool(ptr::eq(p_scaled_1, p_slice_1));

    unsafe {
        let arr32 = black_box([10u32, 20, 30, 40]);
        let s1 = arr32.as_ptr().add(1);
        print_bool(ptr::eq(s1, black_box(s1.cast::<()>()).cast::<u32>()));
        let v16 = black_box(vec![11u16, 22, 33]);
        let h1 = v16.as_ptr().add(1);
        print_bool(ptr::eq(h1, black_box(h1.cast::<()>()).cast::<u16>()));
        let la1 = Layout::from_size_align(8, 1).unwrap();
        let aa1 = alloc(la1);
        let pa1 = aa1 as *mut u16;
        *pa1.add(1) = 44;
        let q1 = pa1.add(1);
        print_bool(ptr::eq(q1, black_box(q1.cast::<()>()).cast::<u16>()));
        print_bool(ptr::eq(aa1.add(2).cast::<()>(), q1.cast::<()>()));
        dealloc(aa1, la1);
    }
}
