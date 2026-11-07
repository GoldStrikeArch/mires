#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// The emitted spelling of every enum shape: how a variant is built, how a discriminant is read,
// and what a match on one switches over. `examples/tests/17_enum_repr.rs` pins the behaviour;
// this pins the text, so that a change to the representation shows up as a reviewable diff rather
// than as a size number.

/// A fieldless enum with no `repr`.
enum Sign {
    Neg,
    Zero,
    Pos,
}

impl Copy for Sign {}

/// A fieldless enum with an integer `repr`, whose value is the discriminant itself.
#[repr(u8)]
enum Letter {
    A = 65,
    B = 66,
}

impl Copy for Letter {}

/// An enum with fieldless, tuple and struct variants at once.
enum Shape {
    Nothing,
    Num(i32),
    Pair(i32, i32),
    Named { w: i32, h: i32 },
}

fn sign_value(s: Sign) -> i32 {
    match s {
        Sign::Neg => -1,
        Sign::Zero => 0,
        Sign::Pos => 1,
    }
}

fn is_not_zero(s: Sign) -> bool {
    match s {
        Sign::Neg | Sign::Pos => true,
        Sign::Zero => false,
    }
}

fn letter_value(l: Letter) -> i32 {
    match l {
        Letter::A => 1,
        Letter::B => 2,
    }
}

fn area(s: &Shape) -> i32 {
    match s {
        Shape::Nothing => 0,
        Shape::Num(n) => *n,
        Shape::Pair(a, b) => *a + *b,
        Shape::Named { w, h } => *w * *h,
    }
}

fn overwrite(s: &mut Shape) {
    *s = Shape::Pair(8, 9);
}

#[no_mangle]
fn rust_entry() {
    print_i32(sign_value(Sign::Pos));
    print_bool(is_not_zero(Sign::Zero));
    print_i32(letter_value(Letter::B));
    print_i32(area(&Shape::Nothing));
    print_i32(area(&Shape::Num(7)));
    print_i32(area(&Shape::Pair(3, 4)));
    print_i32(area(&Shape::Named { w: 5, h: 6 }));
    let mut s = Shape::Nothing;
    overwrite(&mut s);
    print_i32(area(&s));
}
