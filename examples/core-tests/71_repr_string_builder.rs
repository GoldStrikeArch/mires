//! The mutable native `String` carrier's contract, exercised end to end (Phase 4, D5).
//!
//! Everything the `{ text, cap, len }` StringBuilder carrier must keep when the representation
//! planner commits it: construction with and without an explicit capacity, UTF-8 byte length
//! (ASCII and multi-byte), capacity growth on `push`/`push_str`, `clear` keeping the capacity,
//! `as_str` borrowing, and the `fmt::Write` sinks. The expected stdout is mode-independent: the
//! baseline lane, the jir lane and every optimizer mode must print exactly this — that is the
//! metamorphic gate the carrier rewrite is held to.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::string::String;
use core::fmt::Write;

fn shout(text: &str) -> usize {
    text.len()
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Construction, byte-exact length and capacity.
    let mut text = String::new();
    print_usize(text.len());
    print_bool(text.capacity() == 0);
    text.push('a');
    text.push('é'); // Two UTF-8 bytes.
    text.push('☃'); // Three UTF-8 bytes.
    print_str(&text);
    print_usize(text.len());
    print_bool(text.capacity() >= 6);

    // An explicit capacity is preserved until growth demands more.
    let mut sized = String::with_capacity(16);
    print_usize(sized.len());
    print_usize(sized.capacity());
    sized.push_str("hello");
    print_usize(sized.capacity());
    sized.push_str(", world!");
    print_str(&sized);
    print_usize(sized.len());

    // `clear` empties the text and keeps the block.
    sized.clear();
    print_usize(sized.len());
    print_bool(sized.capacity() >= 16);

    // `as_str` and a borrow into an ordinary `&str` function.
    sized.push_str("carrier");
    print_usize(shout(sized.as_str()));

    // The `fmt::Write` sinks: `write_str` and `write_char` through the trait.
    let mut sink = String::new();
    sink.write_str("w=").unwrap();
    sink.write_char('9').unwrap();
    sink.write_char('°').unwrap();
    print_str(&sink);
    print_usize(sink.len());

    // Drop order: both builders die here; nothing observable remains.
}
