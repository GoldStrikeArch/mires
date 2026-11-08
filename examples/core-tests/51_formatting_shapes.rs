//! The `format_args!` template shapes the formatting analysis classifies.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::{String, ToString};

#[path = "prelude.rs"]
mod prelude;

#[no_mangle]
pub fn format_integers(width: u32, height: i32) -> String {
    alloc::format!("{width}x{height}")
}

#[no_mangle]
pub fn format_mixed(name: &str, depth: u32) -> String {
    alloc::format!("({name}: {depth})")
}

#[no_mangle]
pub fn format_char_and_bool(mark: char, flag: bool) -> String {
    alloc::format!("{mark}{flag}!")
}

#[no_mangle]
pub fn format_float(value: f64) -> String {
    alloc::format!("{value}px")
}

#[no_mangle]
pub fn format_owned(value: &String) -> String {
    alloc::format!("[{value}]")
}

/// Rejected before any argument is classified: the template asks for a width.
#[no_mangle]
pub fn format_width_spec(value: u32) -> String {
    alloc::format!("{value:>8}")
}

/// A `to_string` result feeding a template, which is the owned-String argument class.
#[no_mangle]
pub fn format_to_string_chain(value: u32) -> String {
    alloc::format!("<{}>", value.to_string())
}

#[no_mangle]
pub fn format_as_str(value: &String) -> &str {
    value.as_str()
}

#[no_mangle]
pub fn rust_entry() {}
