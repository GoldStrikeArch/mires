//! Idiomatic integer, `bool`, and `char` `to_string` returning native JavaScript strings.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::{String, ToString};

#[path = "prelude.rs"]
mod prelude;

#[no_mangle]
pub fn native_i32_string(value: i32) -> String {
    value.to_string()
}

#[no_mangle]
pub fn native_u8_string(value: u8) -> String {
    value.to_string()
}

#[no_mangle]
pub fn native_i64_string(value: i64) -> String {
    value.to_string()
}

#[no_mangle]
pub fn native_u64_string(value: u64) -> String {
    value.to_string()
}

#[no_mangle]
pub fn native_usize_string(value: usize) -> String {
    value.to_string()
}

#[no_mangle]
pub fn native_bool_string(value: bool) -> String {
    value.to_string()
}

#[no_mangle]
pub fn native_char_string(value: char) -> String {
    value.to_string()
}

#[no_mangle]
pub fn native_int_string_as_str(value: &String) -> &str {
    value.as_str()
}

#[no_mangle]
pub fn native_int_string_len(value: &String) -> usize {
    value.len()
}

#[no_mangle]
pub fn rust_entry() {}
