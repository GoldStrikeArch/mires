//! An idiomatic `String` that crosses exports as an opaque native JavaScript value.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use core::fmt::Write as _;

#[path = "prelude.rs"]
mod prelude;

#[no_mangle]
pub fn native_string() -> String {
    let mut value = String::new();
    value.push('\u{1f980}');
    value.push_str(" ");
    if value.write_char('r').is_err() {
        return value;
    }
    if value.write_str("ust").is_err() {
        return value;
    }
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
pub fn native_string_as_str(value: &String) -> &str {
    value.as_str()
}

#[no_mangle]
pub fn native_string_round_trip(value: String) -> String {
    value
}

#[no_mangle]
pub fn rust_entry() {}
