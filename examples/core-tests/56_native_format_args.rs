//! Native `format!("{x}")` for proven default-Display primitives.
//!
//! `js-opt=on` concatenates static template pieces with already-lowered argument
//! texts and wraps the result in the same native `String` layout `String::new` /
//! `with_capacity` use. The emit must not pull `flt2dec` or `pad_integral`.
//! Width, Debug, and user `Display` stay on baseline `core::fmt`.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;

#[path = "prelude.rs"]
mod prelude;

#[no_mangle]
pub fn format_u32(x: u32) -> String {
    alloc::format!("x={x}")
}

#[no_mangle]
pub fn format_str(x: &str) -> String {
    alloc::format!("x={x}")
}

#[no_mangle]
pub fn format_f64(x: f64) -> String {
    alloc::format!("x={x}")
}

#[no_mangle]
pub fn format_as_str(value: &String) -> &str {
    value.as_str()
}

#[no_mangle]
pub fn rust_entry() {}
