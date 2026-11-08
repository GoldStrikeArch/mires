//! Idiomatic `f64::to_string` returning an opaque native JavaScript `String`.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::{String, ToString};

#[path = "prelude.rs"]
mod prelude;

#[no_mangle]
pub fn native_f64_string(value: f64) -> String {
    value.to_string()
}

#[no_mangle]
pub fn native_f64_string_as_str(value: &String) -> &str {
    value.as_str()
}

#[no_mangle]
pub fn native_f64_string_len(value: &String) -> usize {
    value.len()
}

#[no_mangle]
pub fn native_f64_string_capacity(value: &String) -> usize {
    value.capacity()
}

#[no_mangle]
pub fn native_f64_string_round_trip(value: String) -> String {
    value
}

#[no_mangle]
pub fn rust_entry() {}
