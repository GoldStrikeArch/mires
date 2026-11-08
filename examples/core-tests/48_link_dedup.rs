//! Monomorphizations that print one body under several names, for the link-time duplicate merge.
//!
//! Three shapes, on purpose:
//!
//! * `widen` is generic over a type it only copies, so every instantiation prints the same body
//!   and merges in the first round;
//! * `fold` calls its closure by name, so two instantiations differ until the two identical
//!   closures merge, and only then become duplicates themselves. That is the fixed point;
//! * `scale` captures, and its closure computes something else, so one of each stays distinct and
//!   the merge cannot be mistaken for "everything collapsed".

#![no_main]
#![no_std]

#[path = "prelude.rs"]
mod prelude;

#[inline(never)]
fn widen<T: Copy + Into<u64>>(value: T) -> u64 {
    let widened: u64 = value.into();
    widened.wrapping_mul(3)
}

#[inline(never)]
fn fold<F: Fn(u32, u32) -> u32>(seed: u32, step: F) -> u32 {
    let mut accumulated = seed;
    let mut index = 0;
    while index < 4 {
        accumulated = step(accumulated, index);
        index += 1;
    }
    accumulated
}

#[no_mangle]
pub fn link_dedup(seed: u32) -> u32 {
    let first = fold(seed, |accumulated, index| accumulated.wrapping_add(index));
    let second = fold(first, |accumulated, index| accumulated.wrapping_add(index));
    let scaled = fold(second, |accumulated, index| {
        accumulated.wrapping_mul(3).wrapping_add(index)
    });
    let widened = widen(seed as u8)
        .wrapping_add(widen(seed as u16))
        .wrapping_add(widen(seed as u32));
    (scaled as u64).wrapping_add(widened) as u32
}

#[no_mangle]
pub fn rust_entry() {}
