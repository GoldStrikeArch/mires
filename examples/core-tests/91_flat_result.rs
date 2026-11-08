//! Private aggregate results travel as scalar lanes and are rebuilt only semantically.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[derive(Clone, Copy)]
struct Pair {
    left: u32,
    right: u32,
}

#[inline(never)]
fn make_pair(seed: u32) -> Pair {
    Pair {
        left: seed.wrapping_add(1),
        right: seed.wrapping_mul(3),
    }
}

#[inline(never)]
fn score(seed: u32) -> u32 {
    let first = make_pair(seed);
    let second = make_pair(first.right);
    first.left.wrapping_add(second.right)
}

#[inline(never)]
fn adjacent(seed: u32) -> u32 {
    make_pair(seed)
        .left
        .wrapping_add(make_pair(seed.wrapping_add(1)).right)
}

#[inline(never)]
fn descend(depth: u32) -> Pair {
    if depth == 0 {
        Pair { left: 1, right: 2 }
    } else {
        let previous = descend(depth - 1);
        Pair {
            left: previous.left + 1,
            right: previous.right + 2,
        }
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(score(5));
    print_u32(adjacent(7));
    print_u32(score(u32::MAX));
    let recursive = descend(5);
    print_u32(recursive.left + recursive.right);
}
