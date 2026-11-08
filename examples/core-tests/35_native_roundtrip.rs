//! Exact opaque round trips for the first native late-IR lowering.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;

use alloc::string::String;
use alloc::vec::Vec;

#[no_mangle]
pub fn round_trip_vec(value: Vec<u32>) -> Vec<u32> {
    value
}

#[no_mangle]
pub fn round_trip_string(value: String) -> String {
    value
}

#[no_mangle]
pub fn rust_entry() {}
