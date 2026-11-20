//! A `static` whose initializer holds a niche-encoded reference: `static DOT: Option<&u8> =
//! Some(&b'.')` (MIRES-019).
//!
//! A `static` is materialized from its evaluated initializer, an allocation decoded by the same
//! reader as a promoted constant, so it met the same misreading: the niche of `Option<&T>` is the
//! pointer, the pointer's bytes are its offset into the allocation it names, and offset zero read
//! as a number is `None`. Every `static` below was emitted as `{ TAG: "None" }`. The pointers name
//! promoted values, another `static` and a function, and sit at the top of the initializer, in an
//! array and in a struct field.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

struct Point {
    x: i32,
    y: u8,
}

struct Config {
    separator: Option<&'static str>,
    width: u8,
}

static BYTE: u8 = 42;

fn answer() -> i32 {
    41
}

static DOT: Option<&u8> = Some(&b'.');
static EMPTY: Option<&str> = Some("");
static WORDS: Option<&[&str]> = Some(&["", "b"]);
static POINT: Option<&Point> = Some(&Point { x: 1, y: 2 });
static TO_STATIC: Option<&u8> = Some(&BYTE);
static TABLE: [Option<&u8>; 3] = [Some(&b'a'), None, Some(&b'c')];
static CONFIG: Config = Config {
    separator: Some(", "),
    width: 4,
};
static FUNCTION: Option<fn() -> i32> = Some(answer);

#[inline(never)]
fn byte(value: Option<&u8>) -> i32 {
    match value {
        Some(byte) => *byte as i32,
        None => -1,
    }
}

#[inline(never)]
fn text_length(value: Option<&str>) -> i32 {
    match value {
        Some(text) => text.len() as i32,
        None => -1,
    }
}

#[inline(never)]
fn words(value: Option<&[&str]>) -> i32 {
    match value {
        Some(words) => words.len() as i32 * 10 + words[1].len() as i32,
        None => -1,
    }
}

#[inline(never)]
fn point(value: Option<&Point>) -> i32 {
    match value {
        Some(point) => point.x * 10 + point.y as i32,
        None => -1,
    }
}

#[inline(never)]
fn separator_width(config: &Config) -> i32 {
    match config.separator {
        Some(separator) => separator.len() as i32 * 10 + config.width as i32,
        None => -1,
    }
}

#[inline(never)]
fn call(value: Option<fn() -> i32>) -> i32 {
    match value {
        Some(function) => function(),
        None => -1,
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_i32(byte(DOT));
    print_bool(DOT == Some(&b'.'));
    print_i32(text_length(EMPTY));
    print_bool(EMPTY == Some(""));
    print_i32(words(WORDS));
    print_i32(point(POINT));
    print_i32(byte(TO_STATIC));
    print_i32(byte(TABLE[black_box(0)]));
    print_i32(byte(TABLE[black_box(1)]));
    print_i32(byte(TABLE[black_box(2)]));
    print_i32(separator_width(&CONFIG));
    print_i32(call(FUNCTION));
}
