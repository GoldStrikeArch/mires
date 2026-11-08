//! A function-value reference keeps the baseline body while a prepared direct caller uses one
//! private native representation version.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::string::String;

#[inline(never)]
fn observed_len(value: &String) -> usize {
    value.len()
}

#[unsafe(no_mangle)]
pub static BASELINE_LEN_CALLBACK: fn(&String) -> usize = observed_len;

#[unsafe(no_mangle)]
pub fn repr_version_result() -> usize {
    let mut value = String::with_capacity(64);
    value.push_str("clone");
    observed_len(&value) + value.capacity()
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_usize(repr_version_result());
}
