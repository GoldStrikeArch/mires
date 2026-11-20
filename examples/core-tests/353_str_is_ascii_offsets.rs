//! `str::is_ascii` at every byte offset and length (MIRES-018).
//!
//! `core::slice::ascii::is_ascii` reads a slice of four bytes or more a word at a time: an
//! unaligned `usize` load at its start, aligned loads through the middle and an unaligned load of
//! the last four bytes. Each load casts the byte pointer to `*const usize`, and the run time
//! answered such a cast over byte storage only at a byte offset that is a multiple of four:
//! `&text[1..].is_ascii()` on a 43-byte text threw "byte offset 39 cannot form a u32 scalar view"
//! at the last word. Every offset and length below is read, over ASCII and non-ASCII text, through
//! a borrowed slice, an owned `String`, and a slice that ends before its text does.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

extern crate alloc;
use alloc::string::String;
use core::hint::black_box;

const ASCII: &str = "The quick brown fox jumps over the lazy dog";
const MIXED: &str = "The qu\u{ef}ck br\u{f6}wn f\u{f6}x jumps \u{fc}ber the l\u{e4}zy d\u{f6}g \u{1f600}!";

#[inline(never)]
fn tail_is_ascii(s: &str) -> bool {
    if s.len() > 1 && s.is_char_boundary(1) {
        s[1..].is_ascii()
    } else {
        true
    }
}

#[inline(never)]
fn is_ascii(s: &str) -> bool {
    s.is_ascii()
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    // The reported program: a 43-byte text sliced at byte offset 1.
    let text = black_box(ASCII);
    print_bool(tail_is_ascii(text));
    print_bool(text[1..].is_ascii());
    print_bool(black_box(MIXED)[1..].is_ascii());

    // Every substring that starts in the first eight bytes, by start: how many are ASCII, and the
    // first end at which one stops being ASCII.
    for source in [ASCII, MIXED] {
        let source = black_box(source);
        for start in 0..8 {
            let mut ascii = 0;
            let mut total = 0;
            let mut first_non_ascii = -1;
            for end in start..=source.len() {
                let Some(part) = source.get(start..end) else {
                    continue;
                };
                total += 1;
                if is_ascii(part) {
                    ascii += 1;
                } else if first_non_ascii < 0 {
                    first_non_ascii = end as i32;
                }
            }
            print_i32(ascii);
            print_i32(total);
            print_i32(first_non_ascii);
        }
    }

    // A non-ASCII byte in each position of a word: head, aligned middle and tail reads.
    let mut hits = 0;
    for at in 0..24u8 {
        let s: String = (0..24u8)
            .map(|i| if i == at { '\u{e9}' } else { char::from(b'a' + i) })
            .collect();
        let s = black_box(s);
        for start in 0..8 {
            if let Some(part) = s.get(start..) {
                if part.is_ascii() == (usize::from(at) < start) {
                    hits += 1;
                }
            }
            if let Some(part) = s.get(..s.len() - start) {
                if part.is_ascii() {
                    hits += 100;
                }
            }
        }
    }
    print_i32(hits);

    // An owned `String` grown across word boundaries.
    let mut owned = String::new();
    let mut lengths_ascii = 0;
    for step in 0..20 {
        owned.push(if step == 13 { '\u{3a9}' } else { 'x' });
        if owned.is_ascii() {
            lengths_ascii += 1;
        }
        if owned[step.min(1)..].is_ascii() {
            lengths_ascii += 100;
        }
    }
    print_i32(lengths_ascii);
    print_bool(String::new().is_ascii());
    print_bool(""[..].is_ascii());
}
