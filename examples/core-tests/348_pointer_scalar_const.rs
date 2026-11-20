//! A named constant whose one scalar is a pointer, used by value: `take(SOME_DOT)` where
//! `const SOME_DOT: Option<&u8> = Some(&b'.')` (MIRES-019).
//!
//! rustc keeps a constant of a scalar layout as that scalar, so an `Option<&u8>`, an
//! `Option<NonNull<u8>>` or an `Option<fn()>` reaches codegen as one pointer with provenance and the
//! type around it. Only a pointer *type* was decoded from such a scalar: the backend reported the
//! others as "the pointer constant `Option<&u8>` is not a reference" when the pointer named a
//! promoted value or a `static`, and handed an `Option<fn() -> i32>` over as the bare function when
//! it named a function, which the callee's `match` then found to be neither variant. Such a scalar
//! is now put back into memory, provenance included, and decoded like that type stored there.
//!
//! Every value is asked three ways: passed to a function that is not inlined, matched with
//! `if let`, and compared with `==` against the same value built at run time.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::ptr::NonNull;

#[derive(PartialEq)]
struct Point {
    x: i32,
    y: u8,
}

static BYTE: u8 = 42;

fn answer() -> i32 {
    41
}

const SOME_DOT: Option<&u8> = Some(&b'.');
const NO_BYTE: Option<&u8> = None;
const SOME_SEVEN: Option<&u32> = Some(&7);
const SOME_EMPTY_WORD: Option<&&str> = Some(&"");
const SOME_PAIR: Option<&(u8, u16)> = Some(&(1, 2));
const SOME_POINT: Option<&Point> = Some(&Point { x: -3, y: 9 });
const SOME_UNIT: Option<&()> = Some(&());
const SOME_STATIC: Option<&u8> = Some(&BYTE);
const STATIC_POINTER: Option<NonNull<u8>> = NonNull::new(&raw const BYTE as *mut u8);
const BARE_STATIC_POINTER: NonNull<u8> = NonNull::new(&raw const BYTE as *mut u8).unwrap();
const SOME_FUNCTION: Option<fn() -> i32> = Some(answer);

#[inline(never)]
fn byte(value: Option<&u8>) -> i32 {
    match value {
        Some(byte) => *byte as i32,
        None => -1,
    }
}

#[inline(never)]
fn word(value: Option<&u32>) -> i32 {
    match value {
        Some(word) => *word as i32,
        None => -1,
    }
}

#[inline(never)]
fn text_length(value: Option<&&str>) -> i32 {
    match value {
        Some(text) => text.len() as i32,
        None => -1,
    }
}

#[inline(never)]
fn pair(value: Option<&(u8, u16)>) -> i32 {
    match value {
        Some(&(a, b)) => a as i32 * 1000 + b as i32,
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
fn unit(value: Option<&()>) -> i32 {
    match value {
        Some(()) => 1,
        None => -1,
    }
}

#[inline(never)]
fn pointee(value: Option<NonNull<u8>>) -> i32 {
    match value {
        Some(pointer) => unsafe { *pointer.as_ptr() as i32 },
        None => -1,
    }
}

#[inline(never)]
fn bare_pointee(pointer: NonNull<u8>) -> i32 {
    unsafe { *pointer.as_ptr() as i32 }
}

#[inline(never)]
fn call(value: Option<fn() -> i32>) -> i32 {
    match value {
        Some(function) => function(),
        None => -1,
    }
}

#[inline(never)]
fn if_let_dot() -> i32 {
    if let Some(byte) = SOME_DOT {
        *byte as i32
    } else {
        -1
    }
}

#[inline(never)]
fn if_let_point() -> i32 {
    if let Some(point) = SOME_POINT {
        point.x * 10 + point.y as i32
    } else {
        -1
    }
}

#[inline(never)]
fn if_let_static() -> i32 {
    if let Some(pointer) = STATIC_POINTER {
        unsafe { *pointer.as_ptr() as i32 }
    } else {
        -1
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_str("passed by value");
    print_i32(byte(SOME_DOT));
    print_i32(byte(NO_BYTE));
    print_i32(word(SOME_SEVEN));
    print_i32(text_length(SOME_EMPTY_WORD));
    print_i32(pair(SOME_PAIR));
    print_i32(point(SOME_POINT));
    print_i32(unit(SOME_UNIT));
    print_i32(byte(SOME_STATIC));
    print_i32(pointee(STATIC_POINTER));
    print_i32(bare_pointee(BARE_STATIC_POINTER));
    print_i32(call(SOME_FUNCTION));

    print_str("if let");
    print_i32(if_let_dot());
    print_i32(if_let_point());
    print_i32(if_let_static());

    print_str("==");
    print_bool(black_box("a.").as_bytes().last() == SOME_DOT);
    print_bool(black_box("a,").as_bytes().last() == SOME_DOT);
    print_bool(black_box(Some(&7u32)) == SOME_SEVEN);
    print_bool(black_box(Some(&"")) == SOME_EMPTY_WORD);
    print_bool(black_box(Some(&"x")) == SOME_EMPTY_WORD);
    print_bool(black_box(Some(&BYTE)) == SOME_STATIC);
    print_bool(SOME_DOT.is_some());
    print_bool(STATIC_POINTER.is_some());
    print_bool(SOME_FUNCTION.is_some());
}
