//! BinaryHeap::pop() and string-key struct-field pointer copy/write_bytes regression.
//!
//! Verifies that `copy`, `copy_nonoverlapping`, and `write_bytes` handle `{ $b: obj, $o: "field" }`
//! pointers (used by `BinaryHeap`'s `Hole::drop` when moving `self.elt` back into the heap buffer)
//! without JS string concatenation (`"elt" + 0 === "elt0"`).

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;

use alloc::collections::BinaryHeap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::ptr;
use prelude::*;

struct Holder {
    tag: u32,
    elt: i32,
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. Primitive BinaryHeap::pop()
    let mut heap = BinaryHeap::from([4_i32, 1, 7, 3, 9, 2]);
    let mut out = Vec::new();
    while let Some(v) = heap.pop() {
        out.push(v);
    }
    print_bool(out == vec![9, 7, 4, 3, 2, 1]);
    for v in &out {
        print_i32(*v);
    }

    // 2. Non-Copy String BinaryHeap::pop()
    let mut sheap = BinaryHeap::from([
        String::from("delta"),
        String::from("alpha"),
        String::from("echo"),
        String::from("charlie"),
        String::from("bravo"),
    ]);
    let mut sout = Vec::new();
    while let Some(s) = sheap.pop() {
        sout.push(s);
    }
    print_bool(sout == vec!["echo", "delta", "charlie", "bravo", "alpha"]);

    // 3. Direct struct-field copy_nonoverlapping, copy, and write_bytes
    let mut h = Holder { tag: 1, elt: 42 };
    let mut dst = [0_i32; 2];
    // SAFETY: `h.elt` and `dst[1]` are valid non-overlapping `i32` places.
    unsafe {
        ptr::copy_nonoverlapping(&h.elt as *const i32, &mut dst[1] as *mut i32, 1);
    }
    print_i32(dst[1]);

    // SAFETY: `dst[1]` and `h.elt` are valid `i32` places.
    unsafe {
        dst[0] = 99;
        ptr::copy(&dst[0] as *const i32, &mut h.elt as *mut i32, 1);
    }
    print_i32(h.elt);
    print_u32(h.tag);

    // SAFETY: zeroing `h.elt` (an `i32`) via `write_bytes` is valid.
    unsafe {
        ptr::write_bytes(&mut h.elt as *mut i32, 0, 1);
    }
    print_i32(h.elt);
}
