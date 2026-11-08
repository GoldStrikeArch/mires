//! Every primitive `to_string` lane the native lowerer supports, for the differential oracle.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::{String, ToString};

#[path = "../core-tests/prelude.rs"]
mod prelude;

#[no_mangle]
pub fn native_i8_string(value: i8) -> String {
    value.to_string()
}

#[no_mangle]
pub fn native_u8_string(value: u8) -> String {
    value.to_string()
}

#[no_mangle]
pub fn native_i16_string(value: i16) -> String {
    value.to_string()
}

#[no_mangle]
pub fn native_u16_string(value: u16) -> String {
    value.to_string()
}

#[no_mangle]
pub fn native_i32_string(value: i32) -> String {
    value.to_string()
}

#[no_mangle]
pub fn native_u32_string(value: u32) -> String {
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
pub fn native_i128_string(value: i128) -> String {
    value.to_string()
}

#[no_mangle]
pub fn native_u128_string(value: u128) -> String {
    value.to_string()
}

#[no_mangle]
pub fn native_isize_string(value: isize) -> String {
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
pub fn rust_entry() {}
