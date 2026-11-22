//! Regression tests for `compare_bytes` cross-signedness / partial length,
//! `Box<T> -> Box<dyn Trait>` and `[u8]::align_to` `ptr::eq`/`addr`, and
//! `slice_from_raw_parts` + `transmute::<*const [u8], &[u8]>` / subslice on retyped `raw`.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::alloc::{alloc, dealloc, Layout};
use alloc::boxed::Box;
use core::hint::black_box;
use core::mem;
use core::ptr;

trait Marker {
    fn id(&self) -> u32;
}

#[allow(dead_code)]
struct Pair {
    a: u32,
    b: u32,
}

impl Marker for Pair {
    fn id(&self) -> u32 {
        self.a
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    // 1. compare_bytes between signed i32(-1) and unsigned u32::MAX must compare exact bytes (0).
    let a: i32 = black_box(-1);
    let b: u32 = black_box(u32::MAX);
    let same_bits = unsafe {
        let pa = (&a as *const i32) as *const u8;
        let pb = (&b as *const u32) as *const u8;
        let mut eq = true;
        for i in 0..4 {
            if *pa.add(i) != *pb.add(i) {
                eq = false;
            }
        }
        let s1 = core::slice::from_raw_parts(pa, 4);
        let s2 = core::slice::from_raw_parts(pb, 4);
        eq && s1 == s2
    };
    unsafe {
        js_log_bool(same_bits);
    }

    // 2. Partial byte comparison (3 bytes out of 4) on u32 must observe exact low 3 bytes.
    let c: u32 = black_box(0xaa22_3344);
    let d: u32 = black_box(0xbb22_3344);
    let partial_eq = unsafe {
        let s1 = core::slice::from_raw_parts((&c as *const u32) as *const u8, 3);
        let s2 = core::slice::from_raw_parts((&d as *const u32) as *const u8, 3);
        s1 == s2
    };
    unsafe {
        js_log_bool(partial_eq);
    }

    // 3. Box<Pair> -> Box<dyn Marker> ptr::eq and addr parity.
    let bx = Box::new(Pair { a: 1, b: 2 });
    let raw_before = &*bx as *const Pair as *const ();
    let dyn_bx: Box<dyn Marker> = bx;
    let raw_after = &*dyn_bx as *const dyn Marker as *const ();
    unsafe {
        js_log_bool(ptr::eq(raw_before, raw_after));
        js_log_bool(raw_before.addr() == raw_after.addr());
    }

    // 4. [u8]::align_to::<u32>() ptr::eq against original byte slice.
    let bytes = black_box([1u8, 2, 3, 4, 5, 6, 7, 8]);
    let (head, mid, _tail) = unsafe { bytes.align_to::<u32>() };
    let expected = unsafe { bytes.as_ptr().add(head.len()) as *const () };
    unsafe {
        js_log_bool(ptr::eq(expected, mid.as_ptr() as *const ()));
    }

    // 5. slice_from_raw_parts(raw, 4) + transmute::<*const [u8], &[u8]> + subslice on retyped raw.
    unsafe {
        let l = Layout::array::<u32>(2).unwrap();
        let raw = alloc(l);
        let p = raw as *mut u32;
        *p = black_box(0x1122_3344);
        *p.add(1) = black_box(0x5566_7788);
        let rs: *const [u8] = ptr::slice_from_raw_parts(raw, 8);
        let s: &[u8] = mem::transmute(rs);
        let sub: &[u8] = &(&*rs)[0..4];
        js_log_bool(s[0] == 0x44 && s[1] == 0x33 && s[4] == 0x88 && sub[0] == 0x44 && sub[3] == 0x11);
        dealloc(raw, l);
    }
}
