//! `core::option`: the combinators, the `?` operator, and the niche layouts.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

fn halve(x: i32) -> Option<i32> {
    if x % 2 == 0 { Some(x / 2) } else { None }
}

fn chain(x: i32) -> Option<i32> {
    let a = halve(x)?;
    let b = halve(a)?;
    Some(b + 1)
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let some = Some(7i32);
    let none: Option<i32> = None;

    // Predicates and unwrapping.
    print_bool(some.is_some());
    print_bool(none.is_none());
    print_i32(some.unwrap());
    print_i32(none.unwrap_or(-1));
    print_i32(none.unwrap_or_else(|| -2));
    print_i32(none.unwrap_or_default());

    // Combinators.
    print_i32(some.map(|x| x * 3).unwrap());
    print_i32(some.and_then(halve).unwrap_or(-1));
    print_i32(some.filter(|x| *x > 10).unwrap_or(-1));
    print_i32(some.or(Some(1)).unwrap());
    print_i32(none.or(Some(1)).unwrap());
    print_bool(some.map_or(false, |x| x == 7));

    // `?` chained through two fallible steps.
    print_i32(chain(8).unwrap());
    print_bool(chain(6).is_none());

    // Equality and matching.
    print_bool(some == Some(7));
    print_bool(some != none);
    print_i32(match none {
        Some(v) => v,
        None => 99,
    });

    // Mutation through `&mut`.
    let mut cell = Some(3i32);
    print_i32(cell.take().unwrap());
    print_bool(cell.is_none());
    cell.replace(4);
    print_i32(cell.unwrap());
    if let Some(v) = cell.as_mut() {
        *v += 10;
    }
    print_i32(cell.unwrap());

    // The niche layout: `Option<&T>` is one pointer, `Option<bool>` one byte.
    let value = 5i32;
    let reference: Option<&i32> = Some(&value);
    print_i32(*reference.unwrap());
    let empty: Option<&i32> = None;
    print_bool(empty.is_none());
    let flag: Option<bool> = Some(false);
    print_bool(flag.unwrap());

    // Nested options.
    let nested: Option<Option<i32>> = Some(Some(2));
    print_i32(nested.flatten().unwrap());
}
