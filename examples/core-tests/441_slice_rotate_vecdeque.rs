//! `[T]::rotate_left` / `rotate_right` and `VecDeque` rotation across scalar, tuple, and owned
//! `String` element types (`MaybeUninit<([usize; 32], [T; 0])> -> *mut T`).

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec;
use core::hint::black_box;

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    // 1. u8 (1-byte element)
    let mut bytes = black_box([0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    bytes.rotate_left(3);
    print_u32(u32::from(bytes[0]));
    print_u32(u32::from(bytes[9]));
    bytes.rotate_right(5);
    print_u32(u32::from(bytes[0]));
    print_u32(u32::from(bytes[9]));

    // 2. usize (matches BufType element `[usize; 32]`!)
    let mut usizes = black_box([10usize, 20, 30, 40, 50, 60, 70, 80]);
    usizes.rotate_left(2);
    print_usize(usizes[0]);
    print_usize(usizes[7]);
    usizes.rotate_right(3);
    print_usize(usizes[0]);
    print_usize(usizes[7]);

    // 3. u64 (8-byte BigInt)
    let mut wide = black_box([1u64, 2, 3, 4, 5, 6]);
    wide.rotate_left(2);
    print_u64(wide[0]);
    print_u64(wide[5]);

    // 4. (i32, i32) aggregate tuple
    let mut pairs = black_box([(1i32, 10i32), (2, 20), (3, 30), (4, 40), (5, 50)]);
    pairs.rotate_right(2);
    print_i32(pairs[0].0);
    print_i32(pairs[0].1);
    print_i32(pairs[4].0);

    // 5. Owned String (non-Copy, Drop)
    let mut names = vec![
        String::from("alpha"),
        String::from("beta"),
        String::from("gamma"),
        String::from("delta"),
    ];
    names.rotate_left(1);
    print_str(&names[0]);
    print_str(&names[3]);
    names.rotate_right(2);
    print_str(&names[0]);

    // 6. VecDeque rotation, indexing, and make_contiguous
    let mut dq = VecDeque::new();
    for i in 0..8i32 {
        dq.push_back(i);
    }
    dq.push_front(100);
    dq.push_front(200);
    dq.rotate_left(3);
    print_i32(dq[0]);
    print_i32(*dq.back().unwrap());
    let slice = dq.make_contiguous();
    print_usize(slice.len());
    print_i32(slice[0]);
    print_i32(slice[slice.len() - 1]);
    dq.rotate_right(4);
    print_i32(dq.pop_front().unwrap());
    print_i32(dq.pop_back().unwrap());
}
