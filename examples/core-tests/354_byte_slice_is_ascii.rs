//! `<[u8]>::is_ascii` over every kind of byte storage, at every offset and length (MIRES-018).
//!
//! The word-at-a-time scan in `core::slice::ascii::is_ascii` reads its first and last word
//! unaligned. A byte buffer read as a word answered only at a multiple of four bytes, so any slice
//! of four bytes or more whose start or end was not on a word boundary threw "byte offset N cannot
//! form a u32 scalar view". The storage differs here (a heap `Vec<u8>`, a stack `[u8; N]`, a
//! `[u8; N]` asked directly, a `&str`'s bytes) and so does the position of the first non-ASCII
//! byte, which is in the head word, an aligned word or the tail word of the scan.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

extern crate alloc;
use alloc::vec::Vec;
use core::hint::black_box;

#[inline(never)]
fn is_ascii(bytes: &[u8]) -> bool {
    bytes.is_ascii()
}

/// For every start in `0..8` and length up to 28: how many sub-slices are ASCII, folded with
/// where they start so a wrong answer at one position cannot cancel another.
fn census(bytes: &[u8]) -> i32 {
    let mut sum = 0;
    for start in 0..8 {
        for len in 0..=(bytes.len() - start).min(28) {
            if is_ascii(&bytes[start..start + len]) {
                sum += 1 + start as i32 * 3 + len as i32 * 7;
            }
        }
    }
    sum
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    for bad in [None, Some(0), Some(1), Some(3), Some(4), Some(6), Some(9), Some(17), Some(35)] {
        // Heap storage.
        let mut heap: Vec<u8> = (0..40u8).map(|i| b'a' + i % 26).collect();
        if let Some(at) = bad {
            heap[at] = 0xc3;
        }
        let heap = black_box(heap);
        // Stack storage with the same bytes.
        let mut stack = [0u8; 40];
        stack.copy_from_slice(&heap);
        let stack = black_box(stack);
        print_i32(census(&heap));
        print_i32(census(&stack));
        print_bool(heap.is_ascii());
        print_bool(heap[1..].is_ascii());
        print_bool(stack[3..39].is_ascii());
    }

    // Arrays asked directly, through the unsizing to `[u8]`.
    let five = black_box(*b"abcde");
    let seven = black_box([b'a', b'b', 0xff, b'd', b'e', b'f', b'g']);
    let nine = black_box(*b"abcdefghi");
    let mut thirteen = [b'z'; 13];
    print_bool(five.is_ascii());
    print_bool(seven.is_ascii());
    print_bool(nine.is_ascii());
    print_bool(black_box(thirteen).is_ascii());
    thirteen[12] = 0x90;
    print_bool(black_box(thirteen).is_ascii());

    // The bytes of a `str`.
    let text = black_box("The quick brown f\u{f6}x jumps over the lazy dog");
    print_i32(census(text.as_bytes()));
    print_bool(text.as_bytes()[1..17].is_ascii());
    print_bool(text.as_bytes()[1..18].is_ascii());

    // A long heap slice: many aligned words between an unaligned head and tail.
    let mut long: Vec<u8> = (0..77u8).map(|i| b' ' + i).collect();
    let mut answers = 0;
    for at in [None, Some(0), Some(31), Some(63), Some(70), Some(76)] {
        if let Some(at) = at {
            long[at] = 0x80;
        }
        for start in 0..8 {
            if long[start..].is_ascii() {
                answers += 1;
            }
            if long[..long.len() - start].is_ascii() {
                answers += 10;
            }
        }
        if let Some(at) = at {
            long[at] = b' ';
        }
    }
    print_i32(answers);
}
