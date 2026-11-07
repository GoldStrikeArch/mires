#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// A slice of one-byte elements, cast to `*const str`.
//
// In real `core` this cast is spelled `[AsciiChar]::as_str`, and it sits on the static call graph
// of every `char` formatted with `Debug` — which is to say of `&s[a..b]` on a `str`, of
// `contains`, of `find` with a `&str` pattern and of the float formatter, all of which reach it
// through `str::slice_error_fail`. Those tests live against real `core`, where the cast is a dozen
// frames down from anything a reader would recognise; a refactor of `cast_str` that dropped the
// arm would be diagnosed there as "formatting broke". So the cast is pinned here too, by hand, at
// mini_core scale, in the shape the backend actually matches on.
//
// The element is a **fieldless `#[repr(u8)]` enum**, which the value model represents as the plain
// number its discriminant is. That is the point of the fixture: the cast has to recognise such an
// enum as a one byte element, and one that only accepted `u8` would spell a string out of
// `undefined`s instead of reporting.

#[repr(u8)]
enum Ascii {
    Space = 32,
    Bang = 33,
    Zero = 48,
    Nine = 57,
    A = 65,
    B = 66,
    H = 72,
    Z = 90,
    LowerA = 97,
    LowerI = 105,
    LowerN = 110,
    LowerQ = 113,
    LowerU = 117,
}

/// `[Ascii]::as_str`: the cast under test, written the way `core` writes it.
fn as_str(letters: &[Ascii]) -> &str {
    let letters_ptr: *const [Ascii] = letters;
    let str_ptr = letters_ptr as *const str;
    unsafe { &*str_ptr }
}

/// The same cast reached through a `transmute` rather than an `as`. That is the other door into
/// the backend's `cast_str`, and it has to give the same answer.
fn as_str_transmuted(letters: &[Ascii]) -> &str {
    unsafe { intrinsics::transmute(letters) }
}

#[no_mangle]
pub fn rust_entry() {
    let greeting = [Ascii::H, Ascii::LowerI, Ascii::Bang];
    print_str(as_str(&greeting));
    print_str(as_str_transmuted(&greeting));
    // The byte length of the decoded string, which is the element count of the buffer only
    // because the elements really are one byte each.
    print_i32(str_len(as_str(&greeting)) as i32);
    print_i32(slice_len(&greeting) as i32);

    // Every element is a distinct tag, so a decoder reading the wrong key could not accidentally
    // agree with this on any of them.
    let alphabet = [Ascii::A, Ascii::B, Ascii::Z, Ascii::Zero, Ascii::Nine];
    print_str(as_str(&alphabet));
    print_i32(str_len(as_str(&alphabet)) as i32);

    // A space is a byte like any other, and is here so the result cannot be mistaken for a single
    // word that some other path might have produced.
    let words = [
        Ascii::LowerA,
        Ascii::Space,
        Ascii::LowerQ,
        Ascii::LowerU,
        Ascii::LowerI,
        Ascii::LowerN,
    ];
    print_str(as_str(&words));

    // The empty slice: a length of zero decodes to the empty string rather than reading anything.
    let none: [Ascii; 0] = [];
    print_str(as_str(&none));
    print_i32(str_len(as_str(&none)) as i32);

    // A subslice, so that the offset the decoder starts reading at is not zero. Slice patterns are
    // how `mini_core` sub-slices (see `14_slices`), and this tail starts at element one.
    let all: &[Ascii] = &greeting;
    let tail = match all {
        [_, rest @ ..] => rest,
        [] => all,
    };
    print_str(as_str(tail));
    print_i32(str_len(as_str(tail)) as i32);
}
