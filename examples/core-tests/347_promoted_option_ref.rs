//! A promoted constant whose niche is a reference: `s.as_bytes().last() == Some(&b'.')` (MIRES-019).
//!
//! rustc promotes the right-hand side of such a comparison to a constant, so `==` receives a
//! reference to an allocation holding a niche-encoded `Option<&u8>`. The niche is the reference
//! itself, and the bytes of a pointer in a constant are only the offset into the allocation it
//! points at, with the allocation named by the provenance beside them. `Some(&b'.')` points at
//! offset zero of its own one-byte allocation, and zero read as a number is `None`'s niche: the
//! constant decoded as `None`, so every such comparison answered `false` for a string ending in
//! `.`, and `None == Some("")` answered `true`. A tag that carries provenance is a pointer, which is
//! never null once it is in bounds, and names the variant that holds it.
//!
//! The same decode covers every layout whose niche is a pointer: a `&str` and a `&[u8]` (whose
//! niche is the data pointer of the wide pointer), a reference to a reference, to a tuple and to a
//! struct, `Result<&T, ()>`, a niche in the second field of a tuple, an `Option<Option<&T>>` (whose
//! outer tag is direct and inner one a pointer), and a `const` struct holding one in a field.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

#[derive(PartialEq)]
struct Point {
    x: i32,
    y: u8,
}

struct Config {
    separator: Option<&'static str>,
    width: u8,
}

const CONFIG: Config = Config {
    separator: Some(""),
    width: 3,
};

#[inline(never)]
fn ends_with_dot(s: &str) -> bool {
    s.as_bytes().last() == Some(&b'.')
}

#[inline(never)]
fn does_not_end_with_hash(s: &str) -> bool {
    s.as_bytes().last() != Some(&b'#')
}

#[inline(never)]
fn last_is_seven(values: &[u32]) -> bool {
    values.last() == Some(&7)
}

#[inline(never)]
fn is_empty_text(text: Option<&str>) -> bool {
    text == Some("")
}

#[inline(never)]
fn is_abc(text: Option<&str>) -> bool {
    text == Some("abc")
}

#[inline(never)]
fn is_one_two_three(bytes: Option<&[u8]>) -> bool {
    bytes == Some(&[1, 2, 3][..])
}

#[inline(never)]
fn first_word_is_empty(words: &[&str]) -> bool {
    words.iter().next() == Some(&"")
}

#[inline(never)]
fn first_pair_is_one_two(pairs: &[(u8, u16)]) -> bool {
    pairs.first() == Some(&(1, 2))
}

#[inline(never)]
fn first_point_is_origin(points: &[Point]) -> bool {
    points.first() == Some(&Point { x: -3, y: 9 })
}

#[inline(never)]
fn is_ok_k(result: Result<&u8, ()>) -> bool {
    result == Ok(&b'k')
}

#[inline(never)]
fn is_tagged_x(value: Option<(u8, &u8)>) -> bool {
    value == Some((1, &b'x'))
}

#[inline(never)]
fn is_some_some_y(value: Option<Option<&u8>>) -> bool {
    value == Some(Some(&b'y'))
}

#[inline(never)]
fn separator_width(config: &Config) -> i32 {
    match config.separator {
        Some(separator) => separator.len() as i32 * 10 + config.width as i32,
        None => -1,
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_str("Option<&u8>");
    print_bool(ends_with_dot(black_box("The quick brown fox jumps over the lazy dog.")));
    print_bool(ends_with_dot(black_box("dog")));
    print_bool(ends_with_dot(black_box("")));
    print_bool(does_not_end_with_hash(black_box("# heading #")));
    print_bool(does_not_end_with_hash(black_box("# heading")));

    print_str("Option<&u32>");
    print_bool(last_is_seven(black_box(&[1, 7])));
    print_bool(last_is_seven(black_box(&[7, 1])));

    print_str("Option<&str>");
    print_bool(is_empty_text(black_box(Some(""))));
    print_bool(is_empty_text(black_box(None)));
    print_bool(is_empty_text(black_box(Some("x"))));
    print_bool(is_abc(black_box(Some("abc"))));
    print_bool(is_abc(black_box(Some("ab"))));

    print_str("Option<&[u8]>");
    print_bool(is_one_two_three(black_box(Some(&[1, 2, 3][..]))));
    print_bool(is_one_two_three(black_box(None)));

    print_str("Option<&&str>");
    print_bool(first_word_is_empty(black_box(&["", "x"])));
    print_bool(first_word_is_empty(black_box(&["y", ""])));

    print_str("Option<&(u8, u16)>");
    print_bool(first_pair_is_one_two(black_box(&[(1, 2), (3, 4)])));
    print_bool(first_pair_is_one_two(black_box(&[(3, 4)])));

    print_str("Option<&Point>");
    print_bool(first_point_is_origin(black_box(&[Point { x: -3, y: 9 }])));
    print_bool(first_point_is_origin(black_box(&[Point { x: 3, y: 9 }])));

    print_str("other pointer niches");
    print_bool(is_ok_k(black_box(Ok(&b'k'))));
    print_bool(is_ok_k(black_box(Err(()))));
    print_bool(is_tagged_x(black_box(Some((1, &b'x')))));
    print_bool(is_tagged_x(black_box(None)));
    print_bool(is_some_some_y(black_box(Some(Some(&b'y')))));
    print_bool(is_some_some_y(black_box(Some(None))));
    print_i32(separator_width(&CONFIG));
}
