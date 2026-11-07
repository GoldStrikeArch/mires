#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// Every observable consequence of how an enum is represented in JavaScript, in one place.
//
// The value model gives an enum one of three shapes, and which one it gets follows from the
// enum's own declaration rather than from how it is used. This fixture pins the *behaviour* of
// all three, so that a change to the shapes has to keep every answer below identical:
//
// * a fieldless enum with an integer `repr` is nothing but its discriminant, so a `transmute`
//   between it and that integer moves a value with no shape change at all, and a buffer of
//   one-byte ones can be read as text;
// * a fieldless enum without one is still nothing but its discriminant, but nothing may assume
//   which number stands for which variant, so only `as` and matching can observe it;
// * an enum with a payload-carrying variant is an aggregate with identity, which is what makes a
//   `&mut` to one a reference to the value itself: a whole-value overwrite through that reference
//   has to be seen by every alias.
//
// The last point is the one worth a fixture of its own. `*s = Shape::Pair(..)` over a value that
// held a *fieldless* variant has to drop the old variant's keys and be visible through every
// other reference to the same place; a representation that let a fieldless variant be a value
// without identity would make that write silently local to the reference.

/// A fieldless enum with no `repr`: the shape whose spelling is nobody's business but the
/// backend's.
enum Sign {
    Neg,
    Zero,
    Pos,
}

impl Copy for Sign {}

/// A fieldless enum with explicit, non-contiguous discriminants and no `repr`. `as` is the one
/// thing that observes the numbers, so it is what pins them.
enum Spaced {
    Low = 5,
    Mid = 10,
    High = 20,
}

impl Copy for Spaced {}

/// A fieldless `#[repr(u8)]` enum: the shape a `transmute` to and from `u8` moves through.
#[repr(u8)]
enum Letter {
    A = 65,
    B = 66,
    Z = 90,
}

impl Copy for Letter {}

/// A fieldless `#[repr(i8)]` enum with negative discriminants, which is the shape `Ordering` has.
#[repr(i8)]
enum Three {
    Less = -1,
    Equal = 0,
    Greater = 1,
}

impl Copy for Three {}

/// An enum with fieldless, tuple and struct variants at once: the aggregate shape.
enum Shape {
    Nothing,
    Num(i32),
    Pair(i32, i32),
    Named { w: i32, h: i32 },
}

/// A two variant enum whose second variant carries a reference, which is the shape `Option<&T>`
/// has and the one a niche layout applies to.
enum Maybe<'a> {
    Nope,
    Ref(&'a i32),
}

fn sign_of(n: i32) -> Sign {
    if n < 0 {
        Sign::Neg
    } else if n == 0 {
        Sign::Zero
    } else {
        Sign::Pos
    }
}

fn sign_value(s: Sign) -> i32 {
    match s {
        Sign::Neg => -1,
        Sign::Zero => 0,
        Sign::Pos => 1,
    }
}

/// An or-pattern over a fieldless enum: two variants share one arm, which is the multi-value test
/// the switch lowering has to spell.
fn is_not_zero(s: Sign) -> bool {
    match s {
        Sign::Neg | Sign::Pos => true,
        Sign::Zero => false,
    }
}

/// Equality of two fieldless enum values, written the way a `PartialEq` derive writes it.
fn same_sign(a: Sign, b: Sign) -> bool {
    match a {
        Sign::Neg => match b {
            Sign::Neg => true,
            _ => false,
        },
        Sign::Zero => match b {
            Sign::Zero => true,
            _ => false,
        },
        Sign::Pos => match b {
            Sign::Pos => true,
            _ => false,
        },
    }
}

/// A write through a `&mut` to a *fieldless* enum, which is a write to a place holding a value
/// with no shape of its own.
fn negate(s: &mut Sign) {
    *s = match *s {
        Sign::Neg => Sign::Pos,
        Sign::Zero => Sign::Zero,
        Sign::Pos => Sign::Neg,
    };
}

fn letter_byte(l: Letter) -> u8 {
    unsafe { intrinsics::transmute(l) }
}

fn byte_letter(n: u8) -> Letter {
    unsafe { intrinsics::transmute(n) }
}

fn letter_value(l: Letter) -> i32 {
    match l {
        Letter::A => 1,
        Letter::B => 2,
        Letter::Z => 26,
    }
}

fn three_value(t: Three) -> i32 {
    match t {
        Three::Less => -1,
        Three::Equal => 0,
        Three::Greater => 1,
    }
}

fn describe(s: &Shape) -> i32 {
    match s {
        Shape::Nothing => 0,
        Shape::Num(n) => *n,
        Shape::Pair(a, b) => *a + *b,
        Shape::Named { w, h } => *w * *h,
    }
}

/// A whole-value overwrite through a reference to an enum. Every other reference to the same
/// place has to see it, including when the old value held a fieldless variant and the new one
/// carries a payload.
fn overwrite_with_pair(s: &mut Shape) {
    *s = Shape::Pair(8, 9);
}

fn overwrite_with_nothing(s: &mut Shape) {
    *s = Shape::Nothing;
}

fn maybe_value(m: &Maybe<'_>) -> i32 {
    match m {
        Maybe::Nope => -1,
        Maybe::Ref(r) => **r,
    }
}

/// A `&mut` to an enum *field*, mutated in place rather than overwritten.
fn double_payload(s: &mut Shape) {
    match s {
        Shape::Nothing => {}
        Shape::Num(n) => *n *= 2,
        Shape::Pair(a, b) => {
            *a *= 2;
            *b *= 2;
        }
        Shape::Named { w, h } => {
            *w += 1;
            *h += 1;
        }
    }
}

#[no_mangle]
pub fn rust_entry() {
    // A fieldless enum built, passed by value and matched.
    print_i32(sign_value(sign_of(-4)));
    print_i32(sign_value(sign_of(0)));
    print_i32(sign_value(sign_of(4)));

    // Or-patterns and equality over the same enum.
    print_bool(is_not_zero(Sign::Neg));
    print_bool(is_not_zero(Sign::Zero));
    print_bool(is_not_zero(Sign::Pos));
    print_bool(same_sign(Sign::Pos, Sign::Pos));
    print_bool(same_sign(Sign::Pos, Sign::Neg));

    // A fieldless enum local whose address is taken, written through the reference.
    let mut s = Sign::Neg;
    negate(&mut s);
    print_i32(sign_value(s));
    negate(&mut s);
    print_i32(sign_value(s));

    // An array of fieldless enums, read back element by element.
    let signs = [Sign::Pos, Sign::Zero, Sign::Neg];
    let mut i = 0;
    let mut total = 0;
    while i < 3 {
        total += sign_value(signs[i]);
        i += 1;
    }
    print_i32(total);

    // `as` on a fieldless enum with explicit discriminants: the numbers themselves.
    print_i32(Spaced::Low as i32);
    print_i32(Spaced::Mid as i32);
    print_i32(Spaced::High as i32);

    // A `#[repr(u8)]` fieldless enum through a transmute in both directions.
    print_i32(letter_byte(Letter::A) as i32);
    print_i32(letter_byte(Letter::Z) as i32);
    print_i32(letter_value(byte_letter(66)));
    print_i32(letter_value(byte_letter(90)));

    // A `#[repr(i8)]` one, whose discriminants are negative.
    print_i32(three_value(Three::Less));
    print_i32(three_value(Three::Equal));
    print_i32(three_value(Three::Greater));
    print_i32(Three::Less as i32);

    // The aggregate shape: every variant kind built and read.
    print_i32(describe(&Shape::Nothing));
    print_i32(describe(&Shape::Num(7)));
    print_i32(describe(&Shape::Pair(3, 4)));
    print_i32(describe(&Shape::Named { w: 5, h: 6 }));

    // A payload mutated in place through a reference.
    let mut m = Shape::Pair(1, 2);
    double_payload(&mut m);
    print_i32(describe(&m));

    // A whole-value overwrite through a reference, seen through another reference to the same
    // place. Fieldless over payload and payload over fieldless, both ways.
    let mut n = Shape::Nothing;
    let alias = &mut n;
    overwrite_with_pair(alias);
    print_i32(describe(alias));
    print_i32(describe(&n));

    let mut o = Shape::Named { w: 2, h: 3 };
    overwrite_with_nothing(&mut o);
    print_i32(describe(&o));
    overwrite_with_pair(&mut o);
    print_i32(describe(&o));

    // A two variant enum carrying a reference.
    let target = 42;
    print_i32(maybe_value(&Maybe::Nope));
    print_i32(maybe_value(&Maybe::Ref(&target)));

    print_str("17_enum_repr ok");
}
