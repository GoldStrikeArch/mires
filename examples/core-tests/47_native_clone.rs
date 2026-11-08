//! One private function shared by baseline and native representation callers.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

#[path = "prelude.rs"]
mod prelude;

struct Pair {
    left: u32,
    right: u32,
}

#[inline(never)]
fn pair_total(pair: &Pair) -> u32 {
    pair.left.wrapping_add(pair.right)
}

#[no_mangle]
pub fn callback_identity(value: u32) -> u32 {
    value
}

#[no_mangle]
pub fn baseline_pair_total(callback: fn(u32) -> u32, seed: u32) -> u32 {
    let pair = Pair {
        left: callback(seed),
        right: 5,
    };
    pair_total(&pair)
}

#[no_mangle]
pub fn native_clone_values() -> Vec<u32> {
    let pair = Pair { left: 1, right: 2 };
    let mut values = Vec::new();
    values.push(pair_total(&pair));
    values.push(5);
    values
}

#[no_mangle]
pub fn native_clone_values_len(values: &Vec<u32>) -> usize {
    values.len()
}

#[no_mangle]
pub fn rust_entry() {}
