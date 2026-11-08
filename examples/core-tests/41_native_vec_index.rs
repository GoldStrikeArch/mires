//! Shared and mutable indexing through an opaque native `Vec<u32>`.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

#[path = "prelude.rs"]
mod prelude;

#[no_mangle]
pub fn native_vec_index_make() -> Vec<u32> {
    let mut values = Vec::new();
    values.push(3);
    values.push(5);
    values
}

#[no_mangle]
pub fn native_vec_index_len(values: &Vec<u32>) -> usize {
    values.len()
}

#[no_mangle]
pub fn native_vec_index_get(values: &Vec<u32>, index: usize) -> u32 {
    values[index]
}

#[no_mangle]
pub fn native_vec_index_set(values: &mut Vec<u32>, index: usize, value: u32) {
    values[index] = value;
}

#[no_mangle]
pub fn native_vec_index_round_trip(values: Vec<u32>) -> Vec<u32> {
    values
}

#[no_mangle]
pub fn rust_entry() {}
