//! Idiomatic `Vec` and `String` capacity construction through native JavaScript values.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

#[path = "prelude.rs"]
mod prelude;

#[no_mangle]
pub fn native_vec_with_capacity() -> Vec<u32> {
    let mut values = Vec::with_capacity(4);
    values.push(7);
    values
}

#[no_mangle]
pub fn native_vec_len(values: &Vec<u32>) -> usize {
    values.len()
}

#[no_mangle]
pub fn native_vec_capacity(values: &Vec<u32>) -> usize {
    values.capacity()
}

#[no_mangle]
pub fn native_string_with_capacity() -> String {
    let mut value = String::with_capacity(12);
    value.push_str("hi");
    value
}

#[no_mangle]
pub fn native_string_len(value: &String) -> usize {
    value.len()
}

#[no_mangle]
pub fn native_string_capacity(value: &String) -> usize {
    value.capacity()
}

#[no_mangle]
pub fn rust_entry() {}
