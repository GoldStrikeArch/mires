//! A small ordinary-Rust call graph whose aggregate ABI must switch as one native region.

#![no_main]
#![no_std]

#[path = "prelude.rs"]
mod prelude;

struct Pair {
    left: u32,
    right: u32,
}

#[inline(never)]
fn advance(pair: &mut Pair, amount: u32) {
    pair.left = pair.left.wrapping_add(amount);
    pair.right = pair.right.wrapping_add(amount.wrapping_mul(2));
}

#[inline(never)]
fn total(pair: &Pair) -> u32 {
    pair.left.wrapping_add(pair.right)
}

#[no_mangle]
pub fn native_region(seed: u32) -> u32 {
    let mut pair = Pair {
        left: seed,
        right: 5,
    };
    advance(&mut pair, 7);
    total(&pair)
}

#[no_mangle]
pub fn rust_entry() {}
