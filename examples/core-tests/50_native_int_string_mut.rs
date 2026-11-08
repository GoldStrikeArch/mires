//! A mutated integer `to_string` result, which keeps the capacity-carrying native String shape.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::{String, ToString};

#[path = "prelude.rs"]
mod prelude;

#[no_mangle]
pub fn native_int_string_pushed(value: i32, suffix: char) -> String {
    let mut text = value.to_string();
    text.push(suffix);
    text
}

#[no_mangle]
pub fn native_char_string_pushed(value: char, suffix: char) -> String {
    let mut text = value.to_string();
    text.push(suffix);
    text
}

#[no_mangle]
pub fn native_mut_string_as_str(value: &String) -> &str {
    value.as_str()
}

#[no_mangle]
pub fn native_mut_string_len(value: &String) -> usize {
    value.len()
}

#[no_mangle]
pub fn native_mut_string_capacity(value: &String) -> usize {
    value.capacity()
}

#[no_mangle]
pub fn rust_entry() {}
