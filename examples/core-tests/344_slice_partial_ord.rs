//! `<`, `<=`, `>` and `>=` on slices, and the sorts that compare with them.
//!
//! `core` writes `PartialOrd::lt` for `[T]` as `as_underlying(self.__chaining_lt(other)) == 1`,
//! where `as_underlying` is a `transmute` of a `ControlFlow<bool>` to the `u8` its niche layout
//! stores (`core/src/slice/cmp.rs`): `Break(false)` is 0, `Break(true)` is 1 and `Continue(())` is
//! the niche, 2. Here an enum is an object, so that transmute is an encode: the empty variant is
//! its niche value and the other is its payload transmuted to the integer. Before it existed the
//! transmute was a zombie, and with it `a < b` on slices and every `sort` of slices.
//!
//! The encode is the general one, the exact mirror of the decode `NonZero::new` already used, so
//! the transmutes at the end pin it for the other two-variant niche enums: `Option<NonZero<u32>>`,
//! `Option<bool>` and `Option<char>`, each round-tripped.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::num::NonZero;
use core::ops::ControlFlow;

#[inline(never)]
fn compare_units(a: &[u16], b: &[u16]) {
    print_bool(a < b);
    print_bool(a <= b);
    print_bool(a > b);
    print_bool(a >= b);
}

#[inline(never)]
fn less_bytes(a: &[u8], b: &[u8]) -> bool {
    a < b
}

#[inline(never)]
fn less_signed(a: &[i32], b: &[i32]) -> bool {
    a < b
}

#[inline(never)]
fn less_strings(a: &[String], b: &[String]) -> bool {
    a < b
}

#[inline(never)]
fn encode_flow(flow: ControlFlow<bool>) -> u8 {
    unsafe { core::mem::transmute::<ControlFlow<bool>, u8>(flow) }
}

#[inline(never)]
fn decode_flow(byte: u8) -> ControlFlow<bool> {
    unsafe { core::mem::transmute::<u8, ControlFlow<bool>>(byte) }
}

#[inline(never)]
fn encode_nonzero(value: Option<NonZero<u32>>) -> u32 {
    unsafe { core::mem::transmute::<Option<NonZero<u32>>, u32>(value) }
}

#[inline(never)]
fn encode_bool(value: Option<bool>) -> u8 {
    unsafe { core::mem::transmute::<Option<bool>, u8>(value) }
}

#[inline(never)]
fn decode_bool(byte: u8) -> Option<bool> {
    unsafe { core::mem::transmute::<u8, Option<bool>>(byte) }
}

#[inline(never)]
fn encode_char(value: Option<char>) -> u32 {
    unsafe { core::mem::transmute::<Option<char>, u32>(value) }
}

fn print_flow(flow: ControlFlow<bool>) {
    match flow {
        ControlFlow::Continue(()) => print_str("continue"),
        ControlFlow::Break(value) => print_bool(value),
    }
}

fn print_units(units: &[u16]) {
    let text: String = char::decode_utf16(units.iter().copied())
        .map(|unit| unit.unwrap_or('?'))
        .collect();
    print_str(&text);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let word: Vec<u16> = "hello".encode_utf16().collect();
    let other: Vec<u16> = "help".encode_utf16().collect();
    compare_units(&word, &word);
    compare_units(&word[..2], &word);
    compare_units(&word, &word[..2]);
    compare_units(&word, &other);
    compare_units(&[], &word);
    compare_units(&[], &[]);

    print_bool(less_bytes(b"abc", b"abd"));
    print_bool(less_bytes(b"abd", b"abc"));
    print_bool(less_bytes(b"ab", b"abc"));
    print_bool(less_signed(&[-1, 5], &[0]));
    print_bool(less_signed(&[3, -7], &[3, -8]));
    let left = [String::from("pear"), String::from("fig")];
    let right = [String::from("pear"), String::from("kiwi")];
    print_bool(less_strings(&left, &right));
    print_bool(less_strings(&right, &left));

    // The sorts compare with `lt`.
    let words: Vec<Vec<u16>> = ["pear", "fig", "apple", "fig", "kiwi"]
        .iter()
        .map(|word| word.encode_utf16().collect())
        .collect();
    let mut views: Vec<&[u16]> = words.iter().map(|word| word.as_slice()).collect();
    views.sort_unstable();
    for view in &views {
        print_units(view);
    }
    let mut owned = words.clone();
    owned.sort();
    owned.dedup();
    for word in &owned {
        print_units(word);
    }
    let mut groups = vec![vec![3u8, 1], vec![2, 9, 9], vec![3], vec![2, 9]];
    groups.sort_unstable_by(|a, b| b.partial_cmp(a).unwrap());
    for group in &groups {
        print_usize(group.len());
    }
    if let Some(largest) = words.iter().max() {
        print_units(largest);
    }

    // The encode in general, each against the decode it mirrors.
    for flow in [
        ControlFlow::Break(false),
        ControlFlow::Break(true),
        ControlFlow::Continue(()),
    ] {
        let byte = encode_flow(flow);
        print_u32(byte as u32);
        print_flow(decode_flow(byte));
    }
    // A value the inliner cannot fold into a constant: a folded `Some(NonZero(5))` constant is
    // read wrong under `JSC_RUST_MIR_INLINE=on`, a separate gap this fixture does not test.
    print_u32(encode_nonzero(NonZero::new(word.len() as u32)));
    print_u32(encode_nonzero(None));
    for value in [Some(false), Some(true), None] {
        let byte = encode_bool(value);
        print_u32(byte as u32);
        match decode_bool(byte) {
            Some(value) => print_bool(value),
            None => print_str("none"),
        }
    }
    print_u32(encode_char(Some('é')));
    print_u32(encode_char(None));
}
