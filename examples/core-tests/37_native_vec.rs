//! An idiomatic `Vec` that crosses exports as an opaque native JavaScript value.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

#[path = "prelude.rs"]
mod prelude;

#[no_mangle]
pub fn native_vec() -> Vec<u32> {
    let mut values = Vec::new();
    values.push(3);
    values.push(5);
    values
}

#[no_mangle]
pub fn native_vec_len(values: &Vec<u32>) -> usize {
    values.len()
}

#[no_mangle]
pub fn rust_entry() {}
