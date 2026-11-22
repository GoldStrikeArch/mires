//! Wide pointer ordering (`<`, `<=`, `>`, `>=`, `Ord::cmp`) must compare data address first and
//! tie-break equal data addresses by metadata (`len` for slices / `*const str` / `*mut str`,
//! vtable / unsized-tail identity for `*const dyn Trait` and unsized structs).

#![no_std]
#![no_main]
#![allow(ambiguous_wide_pointer_comparisons)]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::cmp::Ordering;
use core::hint::black_box;
use core::ptr::{self, NonNull};

const SHORT: *const [()] = ptr::slice_from_raw_parts(NonNull::<()>::dangling().as_ptr(), 1);
const LONG: *const [()] = ptr::slice_from_raw_parts(NonNull::<()>::dangling().as_ptr(), 2);
const EMPTY: *const [()] = ptr::slice_from_raw_parts(NonNull::<()>::dangling().as_ptr(), 0);

#[inline(never)]
fn compare_zst_slices(short: *const [()], long: *const [()]) {
    print_i32(ptr::eq(short, long) as i32);
    print_i32((short < long) as i32);
    print_i32((short <= long) as i32);
    print_i32((short > long) as i32);
    print_i32((short >= long) as i32);
    print_i32((long > short) as i32);
    print_ordering(short.cmp(&long));
    print_ordering(long.cmp(&short));
    print_ordering(short.cmp(&short));
}

#[inline(never)]
fn check_order_laws<T: ?Sized>(a: *const T, b: *const T) {
    let ab = a.cmp(&b);
    let ba = b.cmp(&a);
    print_bool(a.cmp(&a) == Ordering::Equal);
    print_bool(ab == ba.reverse());
    print_bool((a < b) == (ab == Ordering::Less));
    print_bool((a <= b) == (ab != Ordering::Greater));
    print_bool((a > b) == (ab == Ordering::Greater));
    print_bool((a >= b) == (ab != Ordering::Less));
    print_bool((a == b) == (ab == Ordering::Equal));
    print_bool((a != b) == (ab != Ordering::Equal));
}

trait Tag {
    fn tag(&self) -> i32;
}

#[repr(C)]
struct Inner {
    v: i32,
}

#[repr(C)]
struct Outer {
    first: Inner,
    second: i32,
}

impl Tag for Inner {
    fn tag(&self) -> i32 {
        self.v
    }
}

impl Tag for Outer {
    fn tag(&self) -> i32 {
        self.first.v + self.second
    }
}

struct ZstA;
struct ZstB;

impl Tag for ZstA {
    fn tag(&self) -> i32 {
        1
    }
}

impl Tag for ZstB {
    fn tag(&self) -> i32 {
        2
    }
}

#[repr(C)]
struct TailStruct<T: ?Sized> {
    head: u32,
    tail: T,
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. Exact `slice_order.rs` repro shape (dangling ZST slices).
    compare_zst_slices(black_box(SHORT), black_box(LONG));

    // 2. Empty vs non-empty ZST slices at same dangling address.
    let empty = black_box(EMPTY);
    let short = black_box(SHORT);
    print_bool(empty < short);
    print_bool(short > empty);
    print_ordering(empty.cmp(&short));
    check_order_laws(empty, short);

    // 3. Bare-address slices: equal addr tie-breaks by len; unequal addr beats len.
    let bare_a: *const [()] = black_box(ptr::slice_from_raw_parts(0x1000 as *const (), 1));
    let bare_b: *const [()] = black_box(ptr::slice_from_raw_parts(0x1000 as *const (), 2));
    let bare_hi: *const [()] = black_box(ptr::slice_from_raw_parts(0x1004 as *const (), 0));
    print_ordering(bare_a.cmp(&bare_b));
    print_ordering(bare_b.cmp(&bare_a));
    print_ordering(bare_b.cmp(&bare_hi));
    print_ordering(bare_hi.cmp(&bare_b));
    check_order_laws(bare_a, bare_b);
    check_order_laws(bare_b, bare_hi);

    // 4. Non-ZST slices: equal ptr + unequal len, unequal ptr + reversed len, empty vs non-empty.
    let arr: [u32; 4] = black_box([10, 20, 30, 40]);
    let s01: *const [u32] = black_box(&arr[..1]);
    let s02: *const [u32] = black_box(&arr[..2]);
    let s12: *const [u32] = black_box(&arr[1..2]);
    let s03: *const [u32] = black_box(&arr[0..3]);
    let s11: *const [u32] = black_box(&arr[1..1]);
    print_bool(s01 < s02);
    print_bool(s02 > s01);
    print_ordering(s01.cmp(&s02));
    print_bool(s12 > s03);
    print_bool(s03 < s12);
    print_ordering(s12.cmp(&s03));
    print_bool(s11 < s12);
    print_ordering(s11.cmp(&s12));
    check_order_laws(s01, s02);
    check_order_laws(s12, s03);

    // Bare-address slice matching `s12`'s address tie-breaks by length using elem_size = 4.
    let s12_addr = s12.cast::<u32>() as usize;
    let s12_bare_long: *const [u32] =
        black_box(ptr::slice_from_raw_parts(s12_addr as *const u32, 5));
    print_ordering(s12.cmp(&s12_bare_long));
    print_ordering(s12_bare_long.cmp(&s12));

    // 5. `*const str` and `*mut str`: equal ptr + unequal len, unequal ptr + reversed len.
    let mut utf8: [u8; 5] = black_box(*b"hello");
    let full_mut: &mut str = unsafe { core::str::from_utf8_unchecked_mut(&mut utf8) };
    let p_base: *mut u8 = full_mut.as_mut_ptr();
    let m_short: *mut str = black_box(ptr::slice_from_raw_parts_mut(p_base, 2) as *mut str);
    let m_long: *mut str = black_box(ptr::slice_from_raw_parts_mut(p_base, 4) as *mut str);
    let m_tail: *mut str =
        black_box(ptr::slice_from_raw_parts_mut(unsafe { p_base.add(1) }, 1) as *mut str);
    let c_short: *const str = black_box(m_short as *const str);
    let c_long: *const str = black_box(m_long as *const str);
    let c_tail: *const str = black_box(m_tail as *const str);

    print_bool(m_short < m_long);
    print_ordering(m_short.cmp(&m_long));
    print_ordering(m_tail.cmp(&m_long));
    print_bool(c_short < c_long);
    print_ordering(c_short.cmp(&c_long));
    print_ordering(c_tail.cmp(&c_long));
    check_order_laws(c_short, c_long);
    check_order_laws(c_tail, c_long);

    // 6. `*const dyn Trait`: equal ptr + same vtable, equal ptr + distinct vtables, ZST dyn ptrs.
    let outer = black_box(Outer {
        first: Inner { v: 7 },
        second: 9,
    });
    let d_outer_1: *const dyn Tag = black_box(&outer as *const Outer as *const dyn Tag);
    let d_outer_2: *const dyn Tag = black_box(&outer as &dyn Tag as *const dyn Tag);
    let d_inner: *const dyn Tag = black_box(&outer.first as *const Inner as *const dyn Tag);
    print_bool(d_outer_1 == d_outer_2);
    print_ordering(d_outer_1.cmp(&d_outer_2));
    print_bool(d_outer_1 != d_inner);
    print_bool(d_outer_1.cmp(&d_inner) != Ordering::Equal);
    print_i32(unsafe { (*d_outer_1).tag() + (*d_inner).tag() });
    check_order_laws(d_outer_1, d_inner);

    let za = black_box(ZstA);
    let zb = black_box(ZstB);
    let dz_a: *const dyn Tag = black_box(&za as *const ZstA as *const dyn Tag);
    let dz_b: *const dyn Tag = black_box(&zb as *const ZstB as *const dyn Tag);
    print_bool(dz_a != dz_b);
    print_bool(dz_a.cmp(&dz_b) != Ordering::Equal);
    check_order_laws(dz_a, dz_b);

    // 7. Unsized struct tail (`TailStruct<[u8]>`): equal ptr + equal tail len, distinct ptrs.
    let ts_a = black_box(TailStruct {
        head: 1,
        tail: [10u8, 20],
    });
    let ts_b = black_box(TailStruct {
        head: 2,
        tail: [10u8, 20, 30, 40],
    });
    let pt_a1: *const TailStruct<[u8]> = black_box(&ts_a as *const TailStruct<[u8]>);
    let pt_a2: *const TailStruct<[u8]> = black_box(&ts_a as *const TailStruct<[u8]>);
    let pt_b: *const TailStruct<[u8]> = black_box(&ts_b as *const TailStruct<[u8]>);
    print_bool(pt_a1 == pt_a2);
    print_ordering(pt_a1.cmp(&pt_a2));
    print_bool(pt_a1 != pt_b);
    print_bool(pt_a1.cmp(&pt_b) != Ordering::Equal);
    check_order_laws(pt_a1, pt_b);
}
