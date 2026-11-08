//! Removing a moved element from an opaque native `Vec<u32>`.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

#[path = "prelude.rs"]
mod prelude;

#[no_mangle]
pub fn native_vec_remove_make() -> Vec<u32> {
    let mut values = Vec::with_capacity(5);
    values.push(3);
    values.push(5);
    values.push(8);
    values
}

#[no_mangle]
pub fn native_vec_remove_len(values: &Vec<u32>) -> usize {
    values.len()
}

#[no_mangle]
pub fn native_vec_remove_capacity(values: &Vec<u32>) -> usize {
    values.capacity()
}

#[no_mangle]
pub fn native_vec_remove_get(values: &Vec<u32>, index: usize) -> u32 {
    values[index]
}

#[no_mangle]
pub fn native_vec_remove(values: &mut Vec<u32>, index: usize) -> u32 {
    values.remove(index)
}

#[no_mangle]
pub fn native_vec_remove_round_trip(values: Vec<u32>) -> Vec<u32> {
    values
}

#[no_mangle]
pub fn rust_entry() {}
