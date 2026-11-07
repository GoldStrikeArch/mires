//! `String`, which has no representation of its own: it is a `Vec<u8>` and nothing else.
//!
//! That is the whole design (`CONTRACT.md`, "Allocation"). A `String`'s bytes live in a heap block
//! of one byte elements, and the JavaScript string only appears when something derefs it to a
//! `&str` -- which `core` does through `from_utf8_unchecked`, and this backend answers with
//! `__rt.bytes_str`. So every operation below is a `Vec<u8>` operation with a decode at the end,
//! and what is pinned is that the round trip is exact.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

fn shout(text: &str) -> usize {
    text.len()
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Building one, byte by byte and piece by piece.
    let mut text = String::from("hello");
    print_str(&text);
    print_usize(text.len());

    text.push_str(", world");
    text.push('!');
    print_str(&text);
    print_usize(text.len());

    // `as_str`, and a borrow into an ordinary `&str` function.
    print_usize(shout(text.as_str()));

    // Concatenation, which reallocates the left hand block in place.
    let joined = String::from("a") + "bc" + "def";
    print_str(&joined);
    print_usize(joined.len());

    // Equality is bytewise, and it has to hold against a literal.
    print_bool(joined == "abcdef");
    print_bool(joined.as_str() == "abcdef");
    print_bool(joined == String::from("abcdef"));
    print_bool(joined == "abcdeg");

    // Slicing and searching reach the same bytes.
    print_str(&joined[1..4]);
    print_bool(joined.contains("cde"));
    print_bool(joined.starts_with("abc"));

    // Characters, including one that is more than a byte of UTF-8.
    let mut count = 0;
    let mut last = 'x';
    for c in joined.chars() {
        count += 1;
        last = c;
    }
    print_i32(count);
    print_i32(last as i32);

    let wide = String::from("aé中");
    print_usize(wide.len());
    let mut points = 0;
    for _c in wide.chars() {
        points += 1;
    }
    print_i32(points);
    print_str(&wide);

    // Into the bytes and back: the block is the same one all along.
    let bytes: Vec<u8> = joined.into_bytes();
    print_usize(bytes.len());
    print_i32(bytes[0] as i32);

    match core::str::from_utf8(&bytes) {
        Ok(back) => {
            print_str(back);
            print_usize(back.len());
        }
        Err(_) => print_str("not utf8"),
    }

    // An invalid byte sequence is rejected rather than decoded.
    let bad: Vec<u8> = vec![0xff, 0xfe];
    print_bool(core::str::from_utf8(&bad).is_err());

    // `to_string` on a `&str` is the copy every other allocation is.
    let copied = "copied".to_string();
    print_str(&copied);
    print_usize(copied.len());
}
