//! `String::from_utf8_lossy` and the `Utf8Chunks` iterator under it, on valid, invalid, mixed,
//! multi-byte and empty input.
//!
//! `Utf8Chunks::next` is a loop with several exits whose control flow graph the structurizer can
//! only express through a nested dispatch `switch`. Its last case is the normal return of the
//! chunk; without the `break` that ends it, that case fell through into the dispatch's `default`
//! guard, so every call that produced a chunk threw `rustc_codegen_js: bad block 1` (MIRES-017).

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::borrow::Cow;
use alloc::string::String;

fn show(bytes: &[u8]) {
    let lossy = String::from_utf8_lossy(bytes);
    print_bool(matches!(lossy, Cow::Borrowed(_)));
    print_usize(lossy.len());
    print_usize(lossy.chars().count());
    for c in lossy.chars() {
        print_u32(c as u32);
    }
    print_str(&lossy);
    let mut chunks = 0usize;
    for chunk in bytes.utf8_chunks() {
        chunks += 1;
        print_str(chunk.valid());
        print_usize(chunk.invalid().len());
    }
    print_usize(chunks);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    show(b"a");
    show(b"\xff");
    show(b"ab\xffcd");
    show(b"caf\xc3\xa9");
    show(b"");
    // An invalid sequence cut short at the end, and two errors in a row.
    show(b"x\xe2\x82");
    show(b"\xc3\x28\xa0\xa1z");
    let owned = String::from_utf8_lossy(b"ok\xf0\x9f\x92\x96!").into_owned();
    print_str(&owned);
    print_usize(owned.len());
}
