//! A copy of an enum whose original is still reachable through a reference.
//!
//! A scalarized copy fixes its variant at the copy site, so it may only read its fields back out
//! of a real object whose variant cannot move. It can: `take()`, `*r = None`, `mem::replace` and
//! any `&mut` callee all give the original another variant behind the copy's back. Every function
//! here copies an `Option` or an enum *after* something could have changed it, and prints what the
//! copy holds — the numbers are the same with the escape analysis on and off.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

/// Empties the option through the reference, so `o` is `None` when the caller copies it.
#[inline(never)]
fn take_it(o: &mut Option<(u32, u32)>) -> u32 {
    match o.take() {
        Some((a, b)) => a + b,
        None => 0,
    }
}

#[inline(never)]
fn copy_after_call(seed: u32) -> u32 {
    let mut option: Option<(u32, u32)> = Some((seed, 7));
    let taken = take_it(&mut option);
    let copy = option;
    let read = match copy {
        Some((a, b)) => a * 100 + b,
        None => 5,
    };
    taken + read
}

/// The same, written through a reference in this very function rather than by a callee.
#[inline(never)]
fn copy_after_store(seed: u32) -> u32 {
    let mut option: Option<(u32, u32)> = Some((seed, 7));
    let slot = &mut option;
    if seed > 3 {
        *slot = None;
    }
    let copy = option;
    match copy {
        Some((a, b)) => a * 100 + b,
        None => 5,
    }
}

/// `mem::replace` hands the old value back and leaves the new one behind the reference.
#[inline(never)]
fn copy_after_replace(seed: u32) -> u32 {
    let mut option: Option<u32> = Some(seed);
    let old = core::mem::replace(&mut option, None);
    let copy = option;
    let read = match copy {
        Some(a) => a * 100,
        None => 5,
    };
    read + old.unwrap_or(0)
}

#[derive(Clone, Copy)]
enum Step {
    One(u32),
    Two(u32, u32),
}

/// A variant change, not just a payload change: the copy must not read `Two`'s second field off a
/// `One` object, nor the other way round.
#[inline(never)]
fn morph(step: &mut Step) {
    if let Step::One(x) = *step {
        *step = Step::Two(x, x + 1);
    }
}

#[inline(never)]
fn copy_after_morph(seed: u32) -> u32 {
    let mut step = Step::One(seed);
    morph(&mut step);
    let copy = step;
    match copy {
        Step::One(x) => x,
        Step::Two(x, y) => x * 1000 + y,
    }
}

/// `take()` inside a loop, where the copy the by-value methods make is loop carried.
#[inline(never)]
fn take_in_loop(seed: u32) -> u32 {
    let mut option: Option<u32> = Some(seed);
    let mut index = 0;
    let mut acc = 0;
    while index < 3 {
        if let Some(value) = option.take() {
            acc += value;
        }
        acc += option.map_or(1, |value| value * 2);
        index += 1;
    }
    acc + option.unwrap_or(7)
}

#[no_mangle]
pub fn rust_entry() {
    print_u32(copy_after_call(3));
    print_u32(copy_after_store(3));
    print_u32(copy_after_store(9));
    print_u32(copy_after_replace(3));
    print_u32(copy_after_morph(3));
    print_u32(take_in_loop(5));
}
